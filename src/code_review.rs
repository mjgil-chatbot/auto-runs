use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ReviewFixMode, ReviewScope};

use super::TaskExecutionReport;

#[path = "code_review/chunking.rs"]
mod chunking;
#[path = "code_review/scope.rs"]
mod scope;

pub const REVIEW_PROFILE_PATH: &str = "docs/review-profile.md";
pub const MAX_REVIEW_CHUNK_LINES: usize = 2_000;
pub const MAX_REVIEW_CHUNK_FILES: usize = 32;
pub const REVIEW_PROFILE_STALE_DAYS: i64 = 30;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverity {
    High,
    Medium,
    Low,
}

impl ReviewSeverity {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, JsonSchema)]
pub struct ReviewFinding {
    pub severity: ReviewSeverity,
    pub confidence: String,
    pub file: String,
    pub line: usize,
    pub dimension: String,
    pub description: String,
    #[serde(default)]
    pub fix_suggestion: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AutomatedToolCheck {
    pub command: String,
    pub status: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ReviewArtifactResponse {
    #[serde(flatten)]
    pub report: TaskExecutionReport,
    #[serde(default)]
    pub findings: Vec<ReviewFinding>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReviewChunkTarget {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReviewTaskMetadata {
    pub scope_description: String,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub chunk: Vec<ReviewChunkTarget>,
    #[serde(default)]
    pub automated_checks: Vec<AutomatedToolCheck>,
    #[serde(default)]
    pub profile_path: String,
    #[serde(default)]
    pub profile_warnings: Vec<String>,
    #[serde(default)]
    pub fix_mode: Option<ReviewFixMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ReviewTaskPlan {
    pub title: String,
    pub prompt: String,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub metadata: Option<ReviewTaskMetadata>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CodeReviewPlanArtifact {
    pub scope_description: String,
    pub output_path: String,
    #[serde(default)]
    pub automated_checks: Vec<AutomatedToolCheck>,
    #[serde(default)]
    pub profile_warnings: Vec<String>,
    #[serde(default)]
    pub review_tasks: Vec<ReviewTaskPlan>,
}

pub fn code_review_doc_path(now: DateTime<Utc>) -> String {
    format!("docs/code-review-{}.md", now.format("%Y-%m-%d"))
}

pub fn describe_scope(scope: &ReviewScope) -> String {
    match scope {
        ReviewScope::Worktree => "Current workspace".to_string(),
        ReviewScope::Diff { base } => format!("Changes since `{}`", base),
        ReviewScope::Paths(paths) => {
            if paths.is_empty() {
                "Specific paths".to_string()
            } else {
                format!("Paths: {}", paths.join(", "))
            }
        }
        ReviewScope::Globs(globs) => {
            if globs.is_empty() {
                "Glob patterns".to_string()
            } else {
                format!("Globs: {}", globs.join(", "))
            }
        }
    }
}

pub fn review_profile_warnings(root: &Path, now: DateTime<Utc>) -> Vec<String> {
    let profile_path = root.join(REVIEW_PROFILE_PATH);
    let metadata = match fs::metadata(&profile_path) {
        Ok(metadata) => metadata,
        Err(err) => {
            return vec![format!(
                "Review profile `{}` is missing or inaccessible ({}); architecture checks may be incomplete.",
                REVIEW_PROFILE_PATH,
                err
            )];
        }
    };

    let modified = match metadata.modified() {
        Ok(modified) => modified,
        Err(_) => {
            return vec![format!(
                "Review profile `{}` could not be timestamp-checked.",
                REVIEW_PROFILE_PATH
            )];
        }
    };
    let modified_at = DateTime::<Utc>::from(modified);
    if now - modified_at > Duration::days(REVIEW_PROFILE_STALE_DAYS) {
        vec![format!(
            "Review profile `{}` is older than {} days (last updated {}).",
            REVIEW_PROFILE_PATH,
            REVIEW_PROFILE_STALE_DAYS,
            modified_at.format("%Y-%m-%d")
        )]
    } else {
        Vec::new()
    }
}

pub fn run_automated_checks(root: &Path) -> Vec<AutomatedToolCheck> {
    let commands = [
        ("cargo check", vec!["check"]),
        (
            "cargo clippy -- -D warnings",
            vec!["clippy", "--", "-D", "warnings"],
        ),
        ("cargo test", vec!["test"]),
        ("cargo fmt --check", vec!["fmt", "--check"]),
    ];
    commands
        .into_iter()
        .map(|(label, args)| run_one_check(root, label, &args))
        .collect()
}

fn run_one_check(root: &Path, label: &str, args: &[&str]) -> AutomatedToolCheck {
    let output = Command::new("cargo").args(args).current_dir(root).output();
    match output {
        Ok(output) => {
            let mut combined = String::new();
            if !output.stdout.is_empty() {
                combined.push_str(&String::from_utf8_lossy(&output.stdout));
            }
            if !output.stderr.is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&String::from_utf8_lossy(&output.stderr));
            }
            // Collect up to 3 significant (non-empty, non-marker) lines and join.
            // This preserves diagnostic context from cargo clippy/test/fmt rather than
            // masking failures behind a generic "command failed" or truncating to 240 chars.
            let significant_lines: Vec<&str> = combined
                .lines()
                .filter(|line| {
                    let trimmed = line.trim();
                    !trimmed.is_empty()
                        && !trimmed.starts_with("=")
                        && !trimmed.starts_with("|")
                        && !trimmed.starts_with("-->")
                        && !trimmed.starts_with("---")
                })
                .take(3)
                .collect();

            let summary = if significant_lines.is_empty() {
                if output.status.success() {
                    "ok".to_string()
                } else {
                    "command failed".to_string()
                }
            } else {
                significant_lines.join("; ")
            };
            AutomatedToolCheck {
                command: label.to_string(),
                status: if output.status.success() {
                    "passed".to_string()
                } else {
                    "failed".to_string()
                },
                summary,
            }
        }
        Err(err) => AutomatedToolCheck {
            command: label.to_string(),
            status: "error".to_string(),
            summary: err.to_string(),
        },
    }
}

pub fn build_review_plan(
    root: &Path,
    scope: &ReviewScope,
    selected_dimensions: Option<&[String]>,
    fix_mode: ReviewFixMode,
    automated_checks: Vec<AutomatedToolCheck>,
    profile_warnings: Vec<String>,
    now: DateTime<Utc>,
) -> Result<CodeReviewPlanArtifact> {
    let (files, mut scope_warnings) = scope::resolve_scope_files(root, scope)?;
    if files.is_empty() {
        anyhow::bail!("review scope produced no reviewable files");
    }

    let scope_description = describe_scope(scope);
    let (chunks, content_cache) = chunking::build_chunks_with_cache(root, &files)?;
    let mut merged_warnings = profile_warnings.clone();
    merged_warnings.extend(scope_warnings.iter().cloned());
    let mut review_tasks = Vec::new();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let dimensions =
            chunking::dimensions_for_chunk_cached(&content_cache, &chunk, selected_dimensions)?;
        let metadata = ReviewTaskMetadata {
            scope_description: scope_description.clone(),
            dimensions: dimensions.clone(),
            chunk: chunk.clone(),
            automated_checks: automated_checks.clone(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: merged_warnings.clone(),
            fix_mode: Some(fix_mode.clone()),
            output_path: None,
        };
        review_tasks.push(ReviewTaskPlan {
            title: chunking::build_chunk_title(index + 1, &chunk),
            prompt: chunking::build_chunk_prompt(&metadata),
            read_scope: chunk
                .iter()
                .map(|target| target.path.clone())
                .chain(std::iter::once(REVIEW_PROFILE_PATH.to_string()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            metadata: Some(metadata),
        });
    }

    let mut warnings = profile_warnings;
    warnings.append(&mut scope_warnings);

    Ok(CodeReviewPlanArtifact {
        scope_description,
        output_path: code_review_doc_path(now),
        automated_checks,
        profile_warnings: warnings,
        review_tasks,
    })
}

pub fn parse_review_response(response: &str) -> Result<ReviewArtifactResponse> {
    let mut parsed =
        if super::lookup_contract(super::CoordinationTaskKind::ReviewArtifacts).is_some() {
            if let Ok(envelope) = super::parse_contract_envelope_from_response(response) {
                let payload = super::review_findings_payload_from_envelope(&envelope)?;
                ReviewArtifactResponse {
                    report: super::typed_envelope_to_legacy_report(&envelope),
                    findings: payload.findings,
                }
            } else {
                let json = super::extract_task_execution_report_json(response)
                    .context("missing JSON object in review task output")?;
                let parsed: ReviewArtifactResponse =
                    serde_json::from_str(json).context("invalid review task JSON")?;
                parsed
            }
        } else {
            let json = super::extract_task_execution_report_json(response)
                .context("missing JSON object in review task output")?;
            let parsed: ReviewArtifactResponse =
                serde_json::from_str(json).context("invalid review task JSON")?;
            parsed
        };
    parsed.report.summary = parsed.report.summary.trim().to_string();
    parsed.report.details = parsed.report.details.trim().to_string();
    parsed.report.changed_files = parsed
        .report
        .changed_files
        .into_iter()
        .map(|value| value.trim().trim_matches('`').to_string())
        .filter(|value| !value.is_empty())
        .collect();
    parsed.report.tests_run = normalize_string_list(parsed.report.tests_run);
    parsed.report.warnings = normalize_string_list(parsed.report.warnings);
    parsed.report.blockers = normalize_string_list(parsed.report.blockers);
    parsed.report.remaining_gaps = normalize_string_list(parsed.report.remaining_gaps);
    parsed.report.design_requirements_satisfied =
        normalize_string_list(parsed.report.design_requirements_satisfied);
    for fix_task in &mut parsed.report.fix_tasks {
        fix_task.acceptance_checks =
            normalize_string_list(std::mem::take(&mut fix_task.acceptance_checks));
    }
    validate_review_response(&parsed)?;
    Ok(parsed)
}

pub fn validate_review_response(parsed: &ReviewArtifactResponse) -> Result<()> {
    if !parsed.report.changed_files.is_empty() {
        anyhow::bail!("review artifact must keep `changed_files` empty");
    }
    for finding in &parsed.findings {
        validate_finding(finding)?;
    }
    Ok(())
}

pub fn validate_fix_mode(
    response: &ReviewArtifactResponse,
    metadata: &ReviewTaskMetadata,
) -> Result<()> {
    let fix_mode = metadata.fix_mode.clone().unwrap_or_default();
    if fix_mode == ReviewFixMode::ReportOnly && !response.report.fix_tasks.is_empty() {
        anyhow::bail!("report-only review tasks must not emit fix_tasks");
    }
    Ok(())
}

pub fn render_review_document(
    workspace_name: &str,
    date: NaiveDate,
    scope_description: &str,
    metadata: &ReviewTaskMetadata,
    artifacts: &[ReviewArtifactResponse],
) -> String {
    let mut findings = Vec::new();
    let mut testing_gaps = BTreeSet::new();
    let mut fix_status = BTreeSet::new();
    let mut verification = BTreeSet::new();

    for warning in &metadata.profile_warnings {
        verification.insert(warning.clone());
    }
    for check in &metadata.automated_checks {
        verification.insert(format!(
            "{}: {} ({})",
            check.command, check.status, check.summary
        ));
    }

    for artifact in artifacts {
        for gap in &artifact.report.remaining_gaps {
            testing_gaps.insert(gap.clone());
        }
        for warning in &artifact.report.warnings {
            verification.insert(warning.clone());
        }
        for task in &artifact.report.fix_tasks {
            fix_status.insert(task.title.clone());
        }
        findings.extend(artifact.findings.iter().cloned());
    }

    findings.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.dimension.cmp(&right.dimension))
            .then_with(|| left.description.cmp(&right.description))
    });

    let mut deduped = Vec::new();
    let mut seen = BTreeSet::new();
    for finding in findings {
        let key = format!(
            "{:?}|{}|{}|{}|{}",
            finding.severity, finding.file, finding.line, finding.dimension, finding.description
        );
        if seen.insert(key) {
            deduped.push(finding);
        }
    }

    let mut doc = String::new();
    doc.push_str(&format!("# Code Review: `{}`\n\n", workspace_name));
    doc.push_str(&format!("Date: {}\n", date.format("%Y-%m-%d")));
    doc.push_str(&format!("Scope: {}\n\n", scope_description));
    doc.push_str("## Findings\n\n");
    if deduped.is_empty() {
        doc.push_str("1. Low: No review findings were recorded.\n");
    } else {
        for (index, finding) in deduped.iter().enumerate() {
            doc.push_str(&format!(
                "{}. {}: {} ({}:{}; dimension: {}; confidence: {})",
                index + 1,
                finding.severity.display_name(),
                finding.description,
                finding.file,
                finding.line,
                finding.dimension,
                finding.confidence
            ));
            if !finding.fix_suggestion.trim().is_empty() {
                doc.push_str(&format!(
                    " Suggested fix: {}",
                    finding.fix_suggestion.trim()
                ));
            }
            doc.push('\n');
        }
    }

    doc.push_str("\n## Testing Gaps\n\n");
    if testing_gaps.is_empty() {
        doc.push_str("- None recorded.\n");
    } else {
        for gap in testing_gaps {
            doc.push_str(&format!("- {}\n", gap));
        }
    }

    if !fix_status.is_empty() {
        doc.push_str("\n## Fix Status\n\n");
        for item in fix_status {
            doc.push_str(&format!("- Generated fix task: {}\n", item));
        }
    }

    doc.push_str("\n## Verification\n\n");
    if verification.is_empty() {
        doc.push_str("- No automated verification metadata was recorded.\n");
    } else {
        for item in verification {
            doc.push_str(&format!("- {}\n", item));
        }
    }

    doc
}

fn validate_finding(finding: &ReviewFinding) -> Result<()> {
    if finding.confidence.trim().is_empty() {
        anyhow::bail!("review finding is missing confidence");
    }
    if finding.file.trim().is_empty() {
        anyhow::bail!("review finding is missing file");
    }
    if finding.line == 0 {
        anyhow::bail!("review finding line must be > 0");
    }
    if finding.dimension.trim().is_empty() {
        anyhow::bail!("review finding is missing dimension");
    }
    if finding.description.trim().is_empty() {
        anyhow::bail!("review finding is missing description");
    }
    Ok(())
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::fs;

    include!("code_review/tests.rs");
}
