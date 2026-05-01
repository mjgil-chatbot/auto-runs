use super::*;
#[allow(unused_imports)]
use crate::config;
#[allow(unused_imports)]
use crate::data::ChatSession;
#[allow(unused_imports)]
use crate::memory;
#[allow(unused_imports)]
use crate::model::ModelProvider;
#[allow(unused_imports)]
use crate::provider_bridge::{
    autonomous_provider_overrides, call_provider_cli_stream_with_overrides,
};
#[allow(unused_imports)]
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use serde_json::Value;
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
#[rustfmt::skip]
pub fn build_provider_task_artifact(task: &CoordinationTask, response: &str) -> Result<TaskArtifact> {
    if let Some(artifact) = build_lightweight_execution_artifact(task, response) {
        return Ok(artifact);
    }
    let review_metadata = load_review_task_metadata(&task.run_id, &task.task_id);
    let typed_envelope = lookup_contract(task.kind)
        .and_then(|_| parse_contract_envelope_from_response(response).ok());
    if task.kind == CoordinationTaskKind::ReviewArtifacts && review_metadata.is_some() {
        let review_response = code_review::parse_review_response(response)?;
        if let Some(metadata) = review_metadata.as_ref() {
            code_review::validate_fix_mode(&review_response, metadata)?;
        }
        if !review_response.report.fix_tasks.is_empty() {
            validate_generated_task_specs(&review_response.report.fix_tasks)?;
        }
        let summary = if review_response.report.summary.trim().is_empty() {
            task.title.clone()
        } else {
            review_response.report.summary.trim().to_string()
        };
        let artifact = TaskArtifact {
            run_id: task.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: task.assigned_provider.clone(),
            task_category: task.task_category,
            fallback_from: task.provider_hint.clone(),
            summary,
            body: serde_json::to_string_pretty(&review_response)?,
            completed_at: Utc::now(),
            report: review_response.report,
        };
        if let Some(envelope) = typed_envelope.as_ref() {
            let _ = persist_artifact_contract_meta(
                &task.run_id,
                &task.task_id,
                envelope.payload_kind.clone(),
                envelope.contract_version,
            );
        }
        return Ok(artifact);
    }

    let mut envelope_meta: Option<(TaskPayloadKind, u32)> = typed_envelope
        .as_ref()
        .map(|envelope| (envelope.payload_kind.clone(), envelope.contract_version));
    let report = match task.kind {
        CoordinationTaskKind::PlanFlatTasks => {
            if let Some(envelope) = typed_envelope.as_ref() {
                let payload = plan_tasks_payload_from_envelope(envelope)?;
                validate_generated_task_specs(&payload.generated_tasks)?;
                let mut report = typed_envelope_to_legacy_report(envelope);
                if report.summary.trim().is_empty() {
                    report.summary = format!(
                        "Planned {} generated task slices.",
                        payload.generated_tasks.len()
                    );
                }
                if report.design_requirements_satisfied.is_empty() {
                    report.design_requirements_satisfied = vec![
                        "Returned machine-readable task slices.".to_string(),
                        "Declared generated task scopes for runtime reconciliation."
                            .to_string(),
                    ];
                }
                report
            } else if let Ok((legacy_report, None)) = try_parse_typed_or_legacy(response) {
                legacy_report
            } else {
                let specs = parse_generated_task_specs(response, None)?;
                validate_generated_task_specs(&specs)?;
                TaskExecutionReport {
                    outcome: CoordinationTaskOutcome::CompletedWithChanges,
                    summary: format!("Planned {} generated task slices.", specs.len()),
                    details: response.trim().to_string(),
                    design_requirements_satisfied: vec![
                        "Returned machine-readable task slices.".to_string(),
                        "Declared generated task scopes for runtime reconciliation."
                            .to_string(),
                    ],
                    ..Default::default()
                }
            }
        }
        CoordinationTaskKind::RalphVerify => {
            let verification = parse_ralph_verification(response, &task.run_id)?;
            TaskExecutionReport {
                outcome: if verification.complete {
                    CoordinationTaskOutcome::CompletedNoChange
                } else {
                    CoordinationTaskOutcome::Partial
                },
                summary: verification.reason.clone(),
                details: verification.reason,
                remaining_gaps: if verification.complete {
                    Vec::new()
                } else {
                    vec!["Completion criteria not yet satisfied.".to_string()]
                },
                ..Default::default()
            }
        }
        CoordinationTaskKind::RalphExecute => {
            let exec_result = parse_ralph_execution(response, &task.run_id)?;
            TaskExecutionReport {
                outcome: exec_result.outcome.clone(),
                summary: exec_result.summary.clone(),
                details: exec_result.reason.clone(),
                changed_files: normalize_path_list(exec_result.changed_files),
                blockers: exec_result.blockers,
                warnings: Vec::new(),
                tests_run: Vec::new(),
                remaining_gaps: Vec::new(),
                design_requirements_satisfied: Vec::new(),
                validation_errors: Vec::new(),
                fix_tasks: Vec::new(),
                failure_kind: None,
                recovery_action: None,
            }
        }
        CoordinationTaskKind::PerfBaseline => perf_baseline_report(response)
            .unwrap_or_else(|err| malformed_task_execution_report(response, &err)),
        CoordinationTaskKind::PerfParse => {
            if let Some(envelope) = typed_envelope.as_ref() {
                let mut report = typed_envelope_to_legacy_report(envelope);
                let payload_errors = validate_typed_payload(envelope);
                if !payload_errors.is_empty() {
                    report.validation_errors.extend(payload_errors);
                }
                report
            } else if let Ok((legacy_report, None)) = try_parse_typed_or_legacy(response) {
                legacy_report
            } else {
                perf_parse_report(response)
                    .unwrap_or_else(|err| malformed_task_execution_report(response, &err))
            }
        }
        CoordinationTaskKind::PerfBench => perf_bench_report(response)
            .unwrap_or_else(|err| malformed_task_execution_report(response, &err)),
        CoordinationTaskKind::PerfEvaluate => perf_evaluate_report(response)
            .unwrap_or_else(|err| malformed_task_execution_report(response, &err)),
        _ => match try_parse_typed_or_legacy(response) {
            Ok((mut r, Some(envelope))) => {
                let payload_errors = validate_typed_payload(&envelope);
                if !payload_errors.is_empty() {
                    for err in &payload_errors {
                        warn!("typed payload validation: {}", err);
                    }
                    r.validation_errors.extend(payload_errors);
                }
                r.changed_files = normalize_path_list(r.changed_files);
                envelope_meta = Some((envelope.payload_kind, envelope.contract_version));
                r
            }
            Ok((r, None)) => r,
            Err(err) => malformed_task_execution_report(response, &err),
        },
    };

    let mut report = report;
    if load_run(&task.run_id)
        .ok()
        .and_then(|run| {
            LightweightWorkflowBehavior::for_run(&run)
                .map(|behavior| behavior.tracks_changed_files())
        })
        .is_some_and(|tracks| !tracks)
    {
        report.changed_files.clear();
    }

    let summary = if report.summary.trim().is_empty() {
        task.title.clone()
    } else {
        report.summary.trim().to_string()
    };
    let body = if let Some(envelope) = typed_envelope.as_ref() {
        match envelope.payload_kind {
            TaskPayloadKind::DesignDoc => design_doc_payload_from_envelope(envelope)
                .map(|payload| payload.markdown)
                .unwrap_or_else(|_| response.trim().to_string()),
            TaskPayloadKind::PlanTasks => plan_tasks_payload_from_envelope(envelope)
                .and_then(|payload| {
                    serde_json::to_string_pretty(&payload.generated_tasks)
                        .context("failed to serialize generated tasks payload")
                })
                .unwrap_or_else(|_| response.trim().to_string()),
            TaskPayloadKind::PerfIdeas => perf_ideas_payload_from_envelope(envelope)
                .and_then(|payload| {
                    serde_json::to_string_pretty(&PerfIdeaList {
                        ideas: payload.ideas,
                    })
                    .context("failed to serialize perf ideas payload")
                })
                .unwrap_or_else(|_| response.trim().to_string()),
            _ => {
                if report.details.trim().is_empty() {
                    response.trim().to_string()
                } else {
                    report.details.clone()
                }
            }
        }
    } else if report.details.trim().is_empty() {
        response.trim().to_string()
    } else {
        report.details.clone()
    };

    let artifact = TaskArtifact {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: task.assigned_provider.clone(),
        task_category: task.task_category,
        fallback_from: task.provider_hint.clone(),
        summary,
        body,
        completed_at: Utc::now(),
        report,
    };
    if let Some((kind, version)) = envelope_meta {
        let _ = persist_artifact_contract_meta(&task.run_id, &task.task_id, kind, version);
    }
    Ok(artifact)
}

fn normalize_report_outcome(value: &mut Value) {
    normalize_provider_response(value);
}

pub fn parse_task_execution_report(response: &str) -> Result<TaskExecutionReport> {
    let json = extract_task_execution_report_json(response)
        .context("missing JSON object in task output")?;
    let mut report_value: Value =
        serde_json::from_str(json).context("invalid task execution report JSON")?;
    normalize_report_outcome(&mut report_value);
    let object = report_value
        .as_object()
        .context("task execution report must be a JSON object")?;
    for required_field in ["outcome", "summary", "details"] {
        if !object.contains_key(required_field) {
            anyhow::bail!(
                "task execution report is missing required field '{}'",
                required_field
            );
        }
    }
    let mut report: TaskExecutionReport =
        serde_json::from_value(report_value).context("invalid task execution report JSON")?;
    report.summary = report.summary.trim().to_string();
    report.details = report.details.trim().to_string();
    report.changed_files = normalize_path_list(report.changed_files);
    report.tests_run = normalize_string_list(report.tests_run);
    report.warnings = normalize_string_list(report.warnings);
    report.blockers = normalize_string_list(report.blockers);
    report.remaining_gaps = normalize_string_list(report.remaining_gaps);
    report.design_requirements_satisfied =
        normalize_string_list(report.design_requirements_satisfied);
    report.validation_errors = normalize_string_list(report.validation_errors);
    for fix_task in &mut report.fix_tasks {
        fix_task.acceptance_checks =
            normalize_string_list(std::mem::take(&mut fix_task.acceptance_checks));
    }

    if report.outcome == CoordinationTaskOutcome::CompletedWithChanges
        && report.changed_files.is_empty()
    {
        warn!(
            "outcome=completed_with_changes but changed_files is empty - treating as completed_no_change"
        );
        report.outcome = CoordinationTaskOutcome::CompletedNoChange;
    }

    Ok(report)
}

pub fn malformed_task_execution_report(response: &str, err: &anyhow::Error) -> TaskExecutionReport {
    let trimmed_response = response.trim();
    let mut details = format!(
        "Rust normalized a malformed provider task report before recording the failure.\n\
         Normalization reason: {}\n",
        err
    );
    if !trimmed_response.is_empty() {
        details.push_str("\nRaw provider response:\n");
        details.push_str(trimmed_response);
    }

    TaskExecutionReport {
        outcome: CoordinationTaskOutcome::Failed,
        summary: "Provider returned a malformed task report.".to_string(),
        details,
        warnings: vec![
            "Recovered malformed provider task output into a failure report.".to_string(),
        ],
        ..Default::default()
    }
}

pub fn extract_task_execution_report_json(body: &str) -> Option<&str> {
    let mut offset = 0usize;

    while offset < body.len() {
        let next_start = body[offset..].find('{')?;
        let start = offset + next_start;
        let json = extract_balanced_json(&body[start..], '{', '}')?;
        if json_has_required_report_fields(json) {
            return Some(json);
        }
        offset = start + json.len();
    }

    None
}

pub fn json_has_required_report_fields(json: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return false;
    };
    let Some(object) = value.as_object() else {
        return false;
    };
    ["outcome", "summary", "details"]
        .iter()
        .all(|field| object.contains_key(*field))
}

pub fn normalize_path_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().trim_matches('`').to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

pub fn materialized_artifact_content_for_path(
    artifact: &TaskArtifact,
    materialized_files: &[MaterializedArtifactFile],
    output_path: &str,
) -> Option<String> {
    let normalized_output = normalize_scope(output_path);
    if let Some(snapshot) = materialized_files
        .iter()
        .find(|file| changed_file_matches_output_path(&file.path, &normalized_output))
    {
        return Some(snapshot.content.clone());
    }

    if body_looks_like_materialized_document(&artifact.body, &normalized_output) {
        return Some(artifact.body.clone());
    }

    None
}

pub fn body_looks_like_materialized_document(body: &str, output_path: &str) -> bool {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    for prefix in [
        "created ",
        "i have created ",
        "the new artifact includes ",
        "the prompt artifact follows ",
        "track 3 follows ",
    ] {
        if lower.starts_with(prefix) {
            return false;
        }
    }

    let is_markdown = Path::new(output_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if !is_markdown {
        return trimmed.len() >= 1000;
    }

    let heading_count = trimmed.matches("\n## ").count()
        + trimmed.matches("\n### ").count()
        + usize::from(trimmed.starts_with("# "))
        + usize::from(trimmed.starts_with("## "));
    heading_count >= 4
}

fn artifact_body_contains_materialized_document(
    body: &str,
    materialization_root: &Path,
    output_path: &str,
) -> Result<bool> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }

    let candidate = if Path::new(output_path).is_absolute() {
        PathBuf::from(output_path)
    } else {
        materialization_root.join(output_path)
    };
    let materialized = match fs::read_to_string(&candidate) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(body_looks_like_materialized_document(body, output_path));
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to read materialized document {}",
                    candidate.display()
                )
            });
        }
    };

    let materialized_trimmed = materialized.trim();
    if materialized_trimmed.is_empty() {
        return Ok(false);
    }

    if trimmed == materialized_trimmed
        || trimmed.contains(materialized_trimmed)
        || materialized_trimmed.contains(trimmed)
    {
        return Ok(true);
    }

    Ok(body_looks_like_materialized_document(body, output_path))
}

pub fn document_path_requires_full_artifact_body(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| matches!(ext, "md" | "txt" | "rst" | "adoc"))
}

#[rustfmt::skip]
pub fn validate_task_artifact(run_id: &str, task: &CoordinationTask, artifact: &TaskArtifact) -> Result<Vec<String>> {
    let materialized_files = capture_artifact_materialized_files(task, artifact);
    validate_task_artifact_with_materialized_files(run_id, task, artifact, &materialized_files)
}

pub fn validate_task_artifact_with_materialized_files(
    run_id: &str,
    task: &CoordinationTask,
    artifact: &TaskArtifact,
    materialized_files: &[MaterializedArtifactFile],
) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    let report = &artifact.report;
    let run = load_run(run_id).ok();
    let lightweight_omits_changed_files = run
        .as_ref()
        .is_some_and(|run| lightweight_task_omits_changed_files(run, task));
    if report.summary.trim().is_empty() {
        errors.push("missing task summary".to_string());
    }
    if !report.blockers.is_empty()
        && !matches!(
            report.outcome,
            CoordinationTaskOutcome::Blocked
                | CoordinationTaskOutcome::Partial
                | CoordinationTaskOutcome::Failed
        )
    {
        errors.push("blockers were reported without a blocked/partial/failed outcome".to_string());
    }
    if report.outcome == CoordinationTaskOutcome::CompletedNoChange
        && !report.changed_files.is_empty()
    {
        errors.push("completed_no_change reported changed files".to_string());
    }
    if task_mutates_workspace(task)
        && report.outcome == CoordinationTaskOutcome::CompletedWithChanges
        && report.changed_files.is_empty()
        && !lightweight_omits_changed_files
    {
        errors.push(
            "workspace-mutating task reported completed_with_changes without changed_files"
                .to_string(),
        );
    }
    if lightweight_omits_changed_files
        && task_mutates_workspace(task)
        && report.outcome == CoordinationTaskOutcome::CompletedWithChanges
        && materialized_files.is_empty()
    {
        errors.push("workspace-mutating lightweight task reported completed_with_changes without materialized outputs".to_string());
    }
    let changed_paths: Vec<&str> = if lightweight_omits_changed_files {
        materialized_files
            .iter()
            .map(|file| file.path.as_str())
            .collect()
    } else {
        report.changed_files.iter().map(String::as_str).collect()
    };
    if !task.write_scope.is_empty() {
        for changed_file in &changed_paths {
            if file_is_within_write_scope(task, changed_file) {
                continue;
            }
            if out_of_scope_change_is_unclaimed(run_id, task, changed_file)? {
                continue;
            }
            errors.push(format!(
                "changed file '{}' is outside declared write scope {:?}",
                changed_file, task.write_scope
            ));
        }
    }
    if task_mutates_workspace(task) {
        let materialization_root = task_runsc_materialization_root(task);
        let skip_stub_marker_scan = task.kind == CoordinationTaskKind::MaterializeDesignDoc
            && run
                .as_ref()
                .is_some_and(|run| run.workflow_mode == AutonomousWorkflowMode::CodeReview);
        for changed_file in &changed_paths {
            if !skip_stub_marker_scan
                && file_contains_stub_markers(&materialization_root, changed_file)?
            {
                errors.push(format!(
                    "changed file '{}' still contains blocked stub markers",
                    changed_file
                ));
            }
        }
        if changed_paths.len() == 1 {
            let output_path = changed_paths[0];
            if document_path_requires_full_artifact_body(output_path)
                && !artifact_body_contains_materialized_document(
                    &artifact.body,
                    &materialization_root,
                    output_path,
                )?
            {
                errors.push(format!(
                    "write task artifact body for '{}' did not contain the full document text",
                    output_path
                ));
            }
        }
    }
    if task.kind == CoordinationTaskKind::MaterializeDesignDoc {
        let materialization_root = task_runsc_materialization_root(task);
        for changed_file in &changed_paths {
            if !changed_file_exists(&materialization_root, changed_file) {
                errors.push(format!(
                    "materialize design doc reported changed file '{}' but it is missing",
                    changed_file
                ));
            }
        }
    }
    if task.kind == CoordinationTaskKind::ReviewArtifacts
        && load_review_task_metadata(&task.run_id, &task.task_id).is_none()
        && (!report.blockers.is_empty() || !report.remaining_gaps.is_empty())
        && report.fix_tasks.is_empty()
    {
        errors
            .push("review found blockers or remaining gaps but did not emit fix_tasks".to_string());
    }
    if task.kind == CoordinationTaskKind::ReviewArtifacts
        && load_review_task_metadata(&task.run_id, &task.task_id).is_none()
        && run
            .as_ref()
            .is_some_and(|run| uses_lightweight_runtime_mode(&run.workflow_mode))
        && !report.fix_tasks.is_empty()
    {
        let expected_task_doc = lightweight_next_task_doc_path_for_run(&load_run(run_id)?);
        if !report
            .changed_files
            .iter()
            .chain(materialized_files.iter().map(|file| &file.path))
            .any(|path| changed_file_matches_output_path(path, &expected_task_doc))
        {
            errors.push(format!(
                "lightweight validation review emitted follow-up tasks without writing the next task list document '{}'",
                expected_task_doc
            ));
        }
    }
    Ok(errors)
}

pub fn task_mutates_workspace(task: &CoordinationTask) -> bool {
    matches!(
        task.kind,
        CoordinationTaskKind::MaterializeDesignDoc
            | CoordinationTaskKind::GeneratedWriteTask
            | CoordinationTaskKind::ApplyApprovedEdits
            | CoordinationTaskKind::RalphExecute
    ) || !task.write_scope.is_empty()
}

pub fn out_of_scope_change_is_unclaimed(
    run_id: &str,
    task: &CoordinationTask,
    changed_file: &str,
) -> Result<bool> {
    let scope = scope_entry_for_changed_file(task, changed_file);
    Ok(!has_conflicting_write_scope(
        run_id,
        &task.task_id,
        &[scope],
    )?)
}

pub fn scope_entry_for_changed_file(task: &CoordinationTask, changed_file: &str) -> String {
    workspace_relative_path(Path::new(&task.cwd), changed_file)
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| normalize_scope(changed_file))
}

pub fn retry_scope_candidates_from_previous_scope_failure(
    run_id: &str,
    task: &CoordinationTask,
) -> Result<Vec<String>> {
    let artifact_path = artifact_result_path(run_id, &task.task_id);
    if !artifact_path.is_file() {
        return Ok(Vec::new());
    }

    let raw = match fs::read_to_string(&artifact_path) {
        Ok(raw) => raw,
        Err(_) => return Ok(Vec::new()),
    };
    let artifact = match serde_json::from_str::<TaskArtifact>(&raw) {
        Ok(artifact) => artifact,
        Err(_) => return Ok(Vec::new()),
    };

    if !artifact_had_scope_validation_error(&artifact) {
        return Ok(Vec::new());
    }

    let mut additions = Vec::new();
    for changed_file in &artifact.report.changed_files {
        if file_is_within_write_scope(task, changed_file) {
            continue;
        }
        let scope = scope_entry_for_changed_file(task, changed_file);
        if !additions.contains(&scope) {
            additions.push(scope);
        }
    }
    Ok(additions)
}

pub fn artifact_had_scope_validation_error(artifact: &TaskArtifact) -> bool {
    artifact
        .report
        .validation_errors
        .iter()
        .any(|error| error.contains("outside declared write scope"))
}

pub fn file_is_within_write_scope(task: &CoordinationTask, changed_file: &str) -> bool {
    let relative = workspace_relative_path(Path::new(&task.cwd), changed_file)
        .unwrap_or_else(|| changed_file.trim_matches('/').to_string());
    let cwd_path = Path::new(&task.cwd);
    task.write_scope.iter().any(|scope| {
        let resolved_scope = resolve_scope_to_cwd(cwd_path, scope);
        scope_allows_relative_path(&resolved_scope, &relative)
    })
}

/// Resolve a scope path to be relative to cwd.
///
/// Scope paths may be specified in different ways:
/// 1. Relative to cwd: "src/lib.rs" → normalized and compared directly
/// 2. Absolute-like path (no leading /): "home/developer/git/repo/Cargo.toml"
///    This is treated as an absolute path "/home/developer/git/repo/Cargo.toml"
///    and resolved to be relative to cwd if cwd is a parent.
pub fn resolve_scope_to_cwd(cwd: &Path, scope: &str) -> String {
    let trimmed = scope.trim_matches('/');

    // First check if scope, as a relative path under cwd, would be valid
    if let Ok(stripped) = cwd.join(trimmed).strip_prefix(cwd) {
        let result = stripped.to_string_lossy().trim_matches('/').to_string();
        // If the result is not the same as what we'd get by treating scope as
        // an absolute path from root, use the relative version
        let as_absolute = Path::new("/").join(trimmed);
        if let Ok(abs_stripped) = as_absolute.strip_prefix(cwd) {
            // Both interpretations work - prefer the shorter one (less nesting)
            let abs_result = abs_stripped.to_string_lossy().trim_matches('/').to_string();
            if result.len() <= abs_result.len() && !result.is_empty() {
                return result;
            }
            return abs_result;
        }
        if !result.is_empty() {
            return result;
        }
    }

    // If that didn't work, scope might be an absolute path from root (no leading /)
    // e.g., "home/developer/git/repo/Cargo.toml" means "/home/developer/git/repo/Cargo.toml"
    if !trimmed.is_empty() && !trimmed.starts_with('.') {
        let as_absolute = Path::new("/").join(trimmed);
        if let Ok(stripped) = as_absolute.strip_prefix(cwd) {
            let result = stripped.to_string_lossy().trim_matches('/').to_string();
            if !result.is_empty() {
                return result;
            }
        }
    }

    // Fallback: return normalized scope
    normalize_scope(trimmed)
}

pub fn execute_builtin_task(task: &CoordinationTask) -> Result<TaskArtifact> {
    match task.kind {
        CoordinationTaskKind::SurveyWorkspace => execute_survey_workspace(task),
        CoordinationTaskKind::PlanCodeReview => execute_plan_code_review(task),
        CoordinationTaskKind::MaterializeDesignDoc
            if load_review_task_metadata(&task.run_id, &task.task_id).is_some() =>
        {
            execute_code_review_materialization(task)
        }
        CoordinationTaskKind::DraftDesignDoc
        | CoordinationTaskKind::ConvergeDesignDoc
        | CoordinationTaskKind::MaterializeDesignDoc
        | CoordinationTaskKind::PlanFlatTasks
        | CoordinationTaskKind::RalphPlan
        | CoordinationTaskKind::GeneratedReadTask
        | CoordinationTaskKind::GeneratedWriteTask
        | CoordinationTaskKind::ReviewArtifacts
        | CoordinationTaskKind::ApplyApprovedEdits
        | CoordinationTaskKind::RalphExecute
        | CoordinationTaskKind::RalphVerify
        | CoordinationTaskKind::PerfBaseline
        | CoordinationTaskKind::PerfParse
        | CoordinationTaskKind::PerfApply
        | CoordinationTaskKind::PerfBench
        | CoordinationTaskKind::PerfEvaluate => {
            anyhow::bail!(
                "provider-backed task {:?} reached builtin executor",
                task.kind
            )
        }
    }
}
