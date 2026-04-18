use super::*;
#[allow(unused_imports)]
use crate::AutonomousWorkflowMode;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use schemars::JsonSchema;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPayloadKind {
    Generic,
    PlanTasks,
    ReviewFindings,
    DesignDoc,
    PerfIdeas,
    PerfBenchmark,
    CouncilControl,
}
impl Default for TaskPayloadKind {
    fn default() -> Self {
        Self::Generic
    }
}
impl TaskPayloadKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::PlanTasks => "plan_tasks",
            Self::ReviewFindings => "review_findings",
            Self::DesignDoc => "design_doc",
            Self::PerfIdeas => "perf_ideas",
            Self::PerfBenchmark => "perf_benchmark",
            Self::CouncilControl => "council_control",
        }
    }}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskArtifactEnvelope {
    pub outcome: CoordinationTaskOutcome,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub changed_files: Option<Vec<String>>,
    #[serde(default)]
    pub tests_run: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub remaining_gaps: Vec<String>,
    #[serde(default)]
    pub design_requirements_satisfied: Vec<String>,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    pub payload_kind: TaskPayloadKind,
    #[serde(default = "default_contract_version")]
    pub contract_version: u32,
    #[serde(default)]
    pub payload: serde_json::Value,
}
fn default_contract_version() -> u32 {
    1
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractResponseMode {
    JsonOnly,
    JsonWithEmbeddedMarkdown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReaskStrategy {
    Full,
    Skeleton,
    Field,
}
#[derive(Clone, Debug)]
pub struct TaskContract {
    pub task_kind: CoordinationTaskKind,
    pub payload_kind: TaskPayloadKind,
    pub contract_version: u32,
    pub max_validation_reasks: u8,
    pub response_mode: ContractResponseMode,
    pub reask_strategy: ReaskStrategy,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct PlanTasksPayload {
    #[serde(default)]
    pub generated_tasks: Vec<GeneratedTaskSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct ReviewFindingsPayload {
    #[serde(default)]
    pub findings: Vec<super::code_review::ReviewFinding>,
    #[serde(default)]
    pub fix_tasks: Vec<GeneratedTaskSpec>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct DesignDocPayload {
    #[serde(default)]
    pub markdown: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default, JsonSchema)]
pub struct PerfIdeasPayload {
    #[serde(default)]
    pub ideas: Vec<PerfIdea>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CouncilControlPayload {
    #[serde(default)]
    pub actions: Vec<super::council::CouncilAction>,
}
static CONTRACT_REGISTRY: std::sync::LazyLock<Vec<TaskContract>> =
    std::sync::LazyLock::new(build_contract_registry);
fn build_contract_registry() -> Vec<TaskContract> {
    vec![
        TaskContract {
            task_kind: CoordinationTaskKind::PlanFlatTasks,
            payload_kind: TaskPayloadKind::PlanTasks,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::PlanCodeReview,
            payload_kind: TaskPayloadKind::PlanTasks,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::ReviewArtifacts,
            payload_kind: TaskPayloadKind::ReviewFindings,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::DraftDesignDoc,
            payload_kind: TaskPayloadKind::DesignDoc,
            contract_version: 1,
            max_validation_reasks: 1,
            response_mode: ContractResponseMode::JsonWithEmbeddedMarkdown,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::ConvergeDesignDoc,
            payload_kind: TaskPayloadKind::DesignDoc,
            contract_version: 1,
            max_validation_reasks: 1,
            response_mode: ContractResponseMode::JsonWithEmbeddedMarkdown,
            reask_strategy: ReaskStrategy::Full,
        },
        TaskContract {
            task_kind: CoordinationTaskKind::PerfParse,
            payload_kind: TaskPayloadKind::PerfIdeas,
            contract_version: 1,
            max_validation_reasks: 2,
            response_mode: ContractResponseMode::JsonOnly,
            reask_strategy: ReaskStrategy::Skeleton,
        },
    ]
}
pub fn lookup_contract(kind: CoordinationTaskKind) -> Option<&'static TaskContract> {
    CONTRACT_REGISTRY.iter().find(|c| c.task_kind == kind)
}
pub fn lookup_contract_for_task(task: &CoordinationTask) -> Option<&'static TaskContract> {
    let contract = lookup_contract(task.kind)?;
    if task.kind != CoordinationTaskKind::PlanFlatTasks {
        return Some(contract);
    }

    let is_branch_fix_plan = load_run(&task.run_id)
        .ok()
        .is_some_and(|run| run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix);
    if is_branch_fix_plan {
        None
    } else {
        Some(contract)
    }
}
pub(crate) fn normalize_provider_response(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(raw) = obj
        .get("outcome")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    {
        let canonical = match raw.trim().to_ascii_lowercase().as_str() {
            "success" | "changes" => Some("completed_with_changes"),
            "complete" | "completed" | "done" | "no_change" | "review_completed" => {
                Some("completed_no_change")
            }
            "blocking" => Some("blocked"),
            "partial" | "partially_complete" | "partially_done" => Some("partial"),
            "failure" | "error" | "fail" => Some("failed"),
            _ => None,
        };
        if let Some(canonical) = canonical {
            warn!(
                "Normalizing non-canonical provider outcome '{}' -> '{}'",
                raw, canonical
            );
            obj.insert(
                "outcome".to_string(),
                serde_json::Value::String(canonical.to_string()),
            );
        }
    }
    if let Some(payload) = obj.get_mut("payload") {
        if let Some(payload_obj) = payload.as_object_mut() {
            if let Some(findings) = payload_obj.get_mut("findings") {
                if let Some(arr) = findings.as_array_mut() {
                    for finding in arr.iter_mut() {
                        normalize_finding_fields(finding);
                    }
                }
            }
        }
    }
}
/// Returns Some((field_name, reason)) if the finding has a validation error.
/// Returns None if the finding is valid.
fn validate_review_finding(
    finding: &super::code_review::ReviewFinding,
) -> Option<(&'static str, &'static str)> {
    use super::code_review::ReviewSeverity;
    match &finding.severity {
        ReviewSeverity::High | ReviewSeverity::Medium | ReviewSeverity::Low => {}
        _ => return Some(("severity", "must be one of: high, medium, low")),
    }
    if finding.confidence.trim().is_empty() {
        return Some(("confidence", "must be a non-empty string"));
    }
    if finding.file.trim().is_empty() {
        return Some(("file", "must be a non-empty string"));
    }
    if finding.line == 0 && finding.file.contains('/') {
        // line 0 is technically valid but suspicious if file is specified
    }
    if finding.dimension.trim().is_empty() {
        return Some(("dimension", "must be a non-empty string"));
    }
    if finding.description.trim().is_empty() {
        return Some(("description", "must be a non-empty string"));
    }
    None
}
fn normalize_finding_fields(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    if let Some(line_val) = obj.get("line") {
        if let Some(s) = line_val.as_str() {
            if let Ok(n) = s.parse::<u64>() {
                obj.insert("line".to_string(), serde_json::Value::Number(n.into()));
            }
        }
    }
}
pub fn parse_typed_envelope(json: &str) -> Result<TaskArtifactEnvelope> {
    let mut value: serde_json::Value =
        serde_json::from_str(json).context("invalid envelope JSON")?;
    let obj = value
        .as_object()
        .context("envelope must be a JSON object")?;
    if !obj.contains_key("payload_kind") {
        anyhow::bail!("missing payload_kind field — not a typed envelope");
    }
    normalize_provider_response(&mut value);
    let envelope: TaskArtifactEnvelope =
        serde_json::from_value(value).context("invalid typed envelope structure")?;
    Ok(envelope)
}
pub fn parse_contract_envelope_from_response(response: &str) -> Result<TaskArtifactEnvelope> {
    let json = extract_first_json_object(response).context("missing JSON object in task output")?;
    parse_typed_envelope(json)
}
pub fn typed_envelope_to_legacy_report(envelope: &TaskArtifactEnvelope) -> TaskExecutionReport {
    let fix_tasks = if envelope.payload_kind == TaskPayloadKind::ReviewFindings {
        review_findings_payload_from_envelope(envelope)
            .map(|payload| payload.fix_tasks)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    TaskExecutionReport {
        outcome: envelope.outcome.clone(),
        summary: envelope.summary.clone(),
        details: envelope.details.clone(),
        changed_files: envelope.changed_files.clone().unwrap_or_default(),
        tests_run: envelope.tests_run.clone(),
        warnings: envelope.warnings.clone(),
        blockers: envelope.blockers.clone(),
        remaining_gaps: envelope.remaining_gaps.clone(),
        design_requirements_satisfied: envelope.design_requirements_satisfied.clone(),
        validation_errors: envelope.validation_errors.clone(),
        fix_tasks,
        failure_kind: None,
        recovery_action: None,
    }
}
pub fn try_parse_typed_or_legacy(
    response: &str,
) -> Result<(TaskExecutionReport, Option<TaskArtifactEnvelope>)> {
    let json = extract_task_execution_report_json(response)
        .context("missing JSON object in task output")?;

    if let Ok(envelope) = parse_typed_envelope(json) {
        let report = typed_envelope_to_legacy_report(&envelope);
        return Ok((report, Some(envelope)));
    }

    let report = parse_task_execution_report(response)?;
    Ok((report, None))
}
pub fn plan_tasks_payload_from_envelope(
    envelope: &TaskArtifactEnvelope,
) -> Result<PlanTasksPayload> {
    serde_json::from_value(envelope.payload.clone()).context("invalid PlanTasks payload")
}
pub fn review_findings_payload_from_envelope(
    envelope: &TaskArtifactEnvelope,
) -> Result<ReviewFindingsPayload> {
    serde_json::from_value(envelope.payload.clone()).context("invalid ReviewFindings payload")
}
pub fn design_doc_payload_from_envelope(
    envelope: &TaskArtifactEnvelope,
) -> Result<DesignDocPayload> {
    serde_json::from_value(envelope.payload.clone()).context("invalid DesignDoc payload")
}
pub fn perf_ideas_payload_from_envelope(
    envelope: &TaskArtifactEnvelope,
) -> Result<PerfIdeasPayload> {
    serde_json::from_value(envelope.payload.clone()).context("invalid PerfIdeas payload")
}
fn normalize_workspace_scope(scope: &str, workspace_root: &Path) -> Option<String> {
    if !workspace_root.is_absolute() {
        return None;
    }
    let trimmed = scope.trim();
    if !raw_scope_is_absolute(trimmed) {
        return None;
    }
    let stripped = Path::new(trimmed).strip_prefix(workspace_root).ok()?;
    Some(normalize_scope(
        &stripped.to_string_lossy().replace('\\', "/"),
    ))
}
fn normalize_generated_task_specs_for_workspace(
    specs: &mut [GeneratedTaskSpec],
    workspace_root: &Path,
) {
    for spec in specs {
        for scope in &mut spec.read_scope {
            if let Some(normalized) = normalize_workspace_scope(scope, workspace_root) {
                *scope = normalized;
            }
        }
        for scope in &mut spec.write_scope {
            if let Some(normalized) = normalize_workspace_scope(scope, workspace_root) {
                *scope = normalized;
            }
        }
    }
}
fn normalize_contract_envelope_for_workspace(
    envelope: &mut TaskArtifactEnvelope,
    workspace_root: &Path,
) -> Result<()> {
    match envelope.payload_kind {
        TaskPayloadKind::PlanTasks => {
            let mut payload = plan_tasks_payload_from_envelope(envelope)?;
            normalize_generated_task_specs_for_workspace(
                &mut payload.generated_tasks,
                workspace_root,
            );
            envelope.payload =
                serde_json::to_value(payload).context("failed to serialize PlanTasks payload")?;
        }
        TaskPayloadKind::ReviewFindings => {
            let mut payload = review_findings_payload_from_envelope(envelope)?;
            normalize_generated_task_specs_for_workspace(&mut payload.fix_tasks, workspace_root);
            envelope.payload = serde_json::to_value(payload)
                .context("failed to serialize ReviewFindings payload")?;
        }
        _ => {}
    }
    Ok(())
}
pub fn validate_contract_envelope(
    contract: &TaskContract,
    envelope: &TaskArtifactEnvelope,
) -> Vec<String> {
    let mut errors = Vec::new();
    if envelope.payload_kind != contract.payload_kind {
        errors.push(format!(
            "invalid task: wrong payload_kind — expected `{}` for {:?}, got `{}` (valid values: plan_tasks, review_findings, design_doc, perf_ideas, generic)",
            contract.payload_kind.as_str(),
            contract.task_kind,
            envelope.payload_kind.as_str()
        ));
    }
    if envelope.contract_version != contract.contract_version {
        errors.push(format!(
            "invalid task: wrong contract_version — expected {}, got {}",
            contract.contract_version, envelope.contract_version
        ));
    }
    errors.extend(validate_typed_payload(envelope));
    errors
}
fn validate_contract_response_inner(
    contract: &TaskContract,
    response: &str,
    workspace_root: Option<&Path>,
) -> Vec<String> {
    match parse_contract_envelope_from_response(response) {
        Ok(mut envelope) => {
            if let Some(workspace_root) = workspace_root {
                if let Err(err) =
                    normalize_contract_envelope_for_workspace(&mut envelope, workspace_root)
                {
                    return vec![format!(
                        "failed to normalize typed `{}` envelope: {}",
                        contract.payload_kind.as_str(),
                        err
                    )];
                }
            }
            validate_contract_envelope(contract, &envelope)
        }
        Err(err) => {
            let mut hints = vec![];
            // Provide structural hints so the provider knows exactly what's missing/wrong
            match err.to_string().as_str() {
                s if s.contains("missing payload_kind") => {
                    hints.push(format!(
                        "the top-level JSON object must include: payload_kind=\"{}\", contract_version={}, outcome, summary, details, and a payload object",
                        contract.payload_kind.as_str(),
                        contract.contract_version
                    ));
                }
                s if s.contains("invalid envelope JSON") => {
                    hints.push(
                        "the response must be a single JSON object at the top level".to_string(),
                    );
                }
                s if s.contains("invalid typed envelope") => {
                    hints.push(format!(
                        "the JSON object must have: payload_kind=\"{}\" (string), contract_version={} (integer), outcome (string), summary (string), details (string), and payload (object with type-specific fields)",
                        contract.payload_kind.as_str(),
                        contract.contract_version
                    ));
                }
                _ => {}
            }
            let mut msg = format!(
                "expected typed `{}` envelope: {}",
                contract.payload_kind.as_str(),
                err
            );
            if !hints.is_empty() {
                msg.push_str(" — ");
                msg.push_str(&hints.join("; "));
            }
            vec![msg]
        }
    }
}
pub fn validate_contract_response(contract: &TaskContract, response: &str) -> Vec<String> {
    validate_contract_response_inner(contract, response, None)
}
pub fn validate_contract_response_for_task(
    contract: &TaskContract,
    response: &str,
    task: &CoordinationTask,
) -> Vec<String> {
    validate_contract_response_inner(contract, response, Some(Path::new(&task.cwd)))
}
pub fn validate_typed_payload(envelope: &TaskArtifactEnvelope) -> Vec<String> {
    let mut errors = Vec::new();
    match envelope.payload_kind {
        TaskPayloadKind::PlanTasks => match plan_tasks_payload_from_envelope(envelope) {
            Ok(payload) => {
                if let Err(err) = validate_generated_task_specs(&payload.generated_tasks) {
                    errors.push(format!("invalid PlanTasks payload: {err}"));
                }
            }
            Err(err) => errors.push(err.to_string()),
        },
        TaskPayloadKind::ReviewFindings => match review_findings_payload_from_envelope(envelope) {
            Ok(payload) => {
                for (i, finding) in payload.findings.iter().enumerate() {
                    if let Some(field) = validate_review_finding(&finding) {
                        errors.push(format!(
                            "invalid task: finding[{}].{} — {}",
                            i, field.0, field.1
                        ));
                    }
                }
                if !payload.fix_tasks.is_empty() {
                    if let Err(err) = validate_generated_task_specs(&payload.fix_tasks) {
                        errors.push(format!("invalid ReviewFindings payload: {err}"));
                    }
                }
            }
            Err(err) => {
                // Give a structured hint about what the payload must contain
                let err_str = err.to_string();
                if err_str.contains("severity") || err_str.contains("ReviewSeverity") {
                    errors.push("invalid task: payload.findings[].severity must be one of: high, medium, low (lowercase string)".to_string());
                } else if err_str.contains("line") {
                    errors.push(
                        "invalid task: payload.findings[].line must be a non-negative integer"
                            .to_string(),
                    );
                } else if err_str.contains("missing field") || err_str.contains("payload") {
                    errors.push("invalid task: payload.findings is missing or malformed — expected payload.findings: [{severity: \"high|medium|low\", confidence: string, file: string, line: uint, dimension: string, description: string, fix_suggestion: string}]".to_string());
                } else {
                    errors.push(format!(
                        "invalid task: ReviewFindings payload error — {}",
                        err
                    ));
                }
            }
        },
        TaskPayloadKind::DesignDoc => match design_doc_payload_from_envelope(envelope) {
            Ok(payload) => {
                if payload.markdown.trim().is_empty() {
                    errors
                        .push("invalid DesignDoc payload: markdown must be non-empty".to_string());
                }
            }
            Err(err) => errors.push(err.to_string()),
        },
        TaskPayloadKind::PerfIdeas => match perf_ideas_payload_from_envelope(envelope) {
            Ok(payload) => {
                if payload.ideas.is_empty() {
                    errors.push("invalid PerfIdeas payload: ideas must be non-empty".to_string());
                } else if payload
                    .ideas
                    .iter()
                    .any(|idea| idea.title.trim().is_empty() || idea.description.trim().is_empty())
                {
                    errors.push(
                        "invalid PerfIdeas payload: each idea must include a title and description"
                            .to_string(),
                    );
                }
            }
            Err(err) => errors.push(err.to_string()),
        },
        TaskPayloadKind::CouncilControl => {
            if let Err(e) =
                serde_json::from_value::<CouncilControlPayload>(envelope.payload.clone())
            {
                errors.push(format!("invalid CouncilControl payload: {e}"));
            }
        }
        TaskPayloadKind::Generic | TaskPayloadKind::PerfBenchmark => {}
    }
    errors
}
