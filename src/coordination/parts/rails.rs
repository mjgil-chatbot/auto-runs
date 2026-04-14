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
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RailStage {
    Input,
    Dialog,
    Execution,
    Output,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum RailDecision {
    Allow,
    Rewrite {
        replacement: String,
    },
    Reject {
        reason: String,
    },
    Reask {
        message: String,
    },
    Interrupt {
        kind: InterruptKind,
        prompt: String,
        #[serde(default)]
        payload: serde_json::Value,
    },
}
impl RailDecision {
    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::Reject { .. } | Self::Interrupt { .. })
    }}
pub fn rail_event(task_id: &str, stage: &RailStage, decision: &RailDecision) -> CoordinationEvent {
    let message = match decision {
        RailDecision::Allow => format!("{stage:?} rail: allow"),
        RailDecision::Rewrite { .. } => format!("{stage:?} rail: rewrite"),
        RailDecision::Reject { reason } => format!("{stage:?} rail: reject - {reason}"),
        RailDecision::Reask { message } => {
            format!(
                "{stage:?} rail: reask - {}",
                message.lines().next().unwrap_or("")
            )
        }
        RailDecision::Interrupt { kind, .. } => format!("{stage:?} rail: interrupt ({kind:?})"),
    };
    CoordinationEvent {
        at: Utc::now(),
        event_type: "rail_decision".to_string(),
        task_id: Some(task_id.to_string()),
        provider: None,
        category: None,
        fallback_from: None,
        message,
    }
}
fn task_requires_existing_design_doc(run: &CoordinationRun, task: &CoordinationTask) -> bool {
    match task.kind {
        CoordinationTaskKind::PlanFlatTasks => {
            !(uses_lightweight_runtime_mode(&run.workflow_mode)
                && task.task_category == Some(TaskCategory::Plan))
        }
        CoordinationTaskKind::ApplyApprovedEdits | CoordinationTaskKind::PerfParse => true,
        _ => false,
    }
}
fn declared_write_scope_reminder(task: &CoordinationTask) -> Option<String> {
    if !task_mutates_workspace(task) || task.write_scope.is_empty() {
        return None;
    }
    Some(format!(
        "Write scope reminder:\n- You may modify files only inside this declared write scope: {}.",
        task.write_scope.join(", ")
    ))
}
fn has_edit_tool(task: &CoordinationTask) -> bool {
    task.allowed_tools
        .iter()
        .any(|tool| tool.eq_ignore_ascii_case("edit"))
}
fn resolved_review_approval_exists(run_id: &str, task: &CoordinationTask) -> bool {
    let dependency_ids: BTreeSet<&str> = task.dependencies.iter().map(String::as_str).collect();
    load_interrupts(run_id).iter().any(|interrupt| {
        interrupt.kind == InterruptKind::ReviewApproval
            && interrupt.status == InterruptStatus::Resolved
            && interrupt.task_id.as_deref().is_some_and(|task_id| {
                dependency_ids.is_empty() || dependency_ids.contains(task_id)
            })
            && matches!(
                interrupt.resolution,
                Some(InterruptResolution::Approved { .. })
                    | Some(InterruptResolution::EditedPayload { .. })
            )
    })
}
fn stage_label(stage: &RailStage) -> &'static str {
    match stage {
        RailStage::Input => "input",
        RailStage::Dialog => "dialog",
        RailStage::Execution => "execution",
        RailStage::Output => "output",
    }
}
fn rail_validation_error(stage: &RailStage, decision: &RailDecision) -> Option<String> {
    let stage = stage_label(stage);
    match decision {
        RailDecision::Allow | RailDecision::Rewrite { .. } => None,
        RailDecision::Reject { reason } => Some(format!("{stage} rail: {reason}")),
        RailDecision::Reask { message } => Some(format!("{stage} rail reask: {message}")),
        RailDecision::Interrupt { kind, prompt, .. } => {
            Some(format!("{stage} rail interrupt ({kind:?}): {prompt}"))
        }
    }
}
pub fn persist_rail_decision(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider_name: &str,
    stage: &RailStage,
    decision: &RailDecision,
) -> Result<()> {
    if matches!(decision, RailDecision::Allow) {
        return Ok(());
    }

    if let RailDecision::Interrupt {
        kind,
        prompt,
        payload,
    } = decision
    {
        let interrupt = CoordinationInterrupt {
            interrupt_id: Uuid::new_v4().to_string(),
            run_id: run.run_id.clone(),
            task_id: Some(task.task_id.clone()),
            council_entry_id: None,
            kind: kind.clone(),
            status: InterruptStatus::Open,
            title: format!("{} rail blocked {}", stage_label(stage), task.title),
            prompt: prompt.clone(),
            payload: payload.clone(),
            created_at: Utc::now(),
            resolved_at: None,
            resolution: None,
        };
        create_interrupt(&interrupt)?;
    }

    let event = rail_event(&task.task_id, stage, decision);
    append_event_with_provider(
        &run.run_id,
        &event.event_type,
        event.task_id.as_deref(),
        Some(provider_name),
        task.task_category,
        task.provider_hint.as_deref(),
        &event.message,
    )
}
pub fn apply_completion_rails(
    run: &CoordinationRun,
    task: &CoordinationTask,
    artifact: &TaskArtifact,
    materialized_files: &[MaterializedArtifactFile],
    provider_name: &str,
) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    for (stage, decision) in [
        (
            RailStage::Execution,
            check_execution_rail(run, task, materialized_files),
        ),
        (
            RailStage::Output,
            check_output_rail(run, task, artifact, materialized_files),
        ),
    ] {
        if matches!(decision, RailDecision::Allow) {
            continue;
        }
        persist_rail_decision(run, task, provider_name, &stage, &decision)?;
        if let Some(error) = rail_validation_error(&stage, &decision) {
            errors.push(error);
        }
    }
    Ok(errors)
}
pub fn apply_lightweight_completion_rails(
    run: &CoordinationRun,
    task: &CoordinationTask,
    _artifact: &TaskArtifact,
    materialized_files: &[MaterializedArtifactFile],
) -> Vec<String> {
    validate_lightweight_task_doc_read_only(run, task, materialized_files)
}
fn validate_lightweight_task_doc_read_only(
    run: &CoordinationRun,
    task: &CoordinationTask,
    materialized_files: &[MaterializedArtifactFile],
) -> Vec<String> {
    if !uses_lightweight_runtime_mode(&run.workflow_mode)
        || task.kind != CoordinationTaskKind::GeneratedWriteTask
    {
        return Vec::new();
    }

    let Some(task_doc_path) =
        lightweight_generated_write_touched_task_doc_path(run, materialized_files)
    else {
        return Vec::new();
    };
    vec![format!(
        "generated lightweight write tasks may not modify task-list document '{}'; review owns task-list progression and next-pass task-list updates",
        task_doc_path
    )]
}
fn lightweight_generated_write_touched_task_doc_path(
    run: &CoordinationRun,
    materialized_files: &[MaterializedArtifactFile],
) -> Option<String> {
    let forbidden_paths = [
        normalize_scope(&lightweight_current_task_doc_path(run)),
        normalize_scope(&lightweight_next_task_doc_path_for_run(run)),
    ];
    materialized_files.iter().find_map(|file| {
        let path = normalize_scope(&file.path);
        forbidden_paths
            .iter()
            .find(|forbidden| **forbidden == path)
            .cloned()
    })
}
pub fn check_input_rail(run: &CoordinationRun, task: &CoordinationTask) -> RailDecision {
    if run.workspace_path.trim().is_empty() {
        return RailDecision::Reject {
            reason: "run is missing workspace_path".to_string(),
        };
    }
    if task.prompt.trim().is_empty() {
        return RailDecision::Reject {
            reason: "empty task prompt".to_string(),
        };
    }
    if task_mutates_workspace(task) && !run.allow_write_tasks {
        return RailDecision::Reject {
            reason: "run disables write tasks but task requires workspace mutation".to_string(),
        };
    }
    if task_mutates_workspace(task) && task.write_scope.is_empty() {
        return RailDecision::Reject {
            reason: "write task has empty write_scope".to_string(),
        };
    }
    if task_requires_existing_design_doc(run, task) {
        if run.design_doc_path.trim().is_empty() {
            return RailDecision::Reject {
                reason: "run is missing design_doc_path".to_string(),
            };
        }
        let design_doc = workspace_path_for_run(run, &run.design_doc_path);
        if !design_doc.is_file() {
            return RailDecision::Reject {
                reason: format!(
                    "required canonical design doc '{}' is missing",
                    run.design_doc_path
                ),
            };
        }
    }
    if let Err(reason) = github_branch_fix_gh_auth_check(run, task) {
        return RailDecision::Reject {
            reason: reason.to_string(),
        };
    }
    RailDecision::Allow
}
fn github_branch_fix_gh_auth_check(run: &CoordinationRun, task: &CoordinationTask) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::GithubBranchFix
        || !matches!(
            task.kind,
            CoordinationTaskKind::GeneratedReadTask
                | CoordinationTaskKind::GeneratedWriteTask
                | CoordinationTaskKind::ReviewArtifacts
        )
    {
        return Ok(());
    }

    let program = std::env::var("AUTO_RUNS_GH_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gh".to_string());
    let active_output = std::process::Command::new(&program)
        .args(["auth", "status", "--active"])
        .output()
        .with_context(|| format!("failed to run `{program} auth status --active`"))?;
    if active_output.status.success() {
        return Ok(());
    }

    let active_stderr = String::from_utf8_lossy(&active_output.stderr)
        .trim()
        .to_string();
    if gh_auth_status_active_unsupported(&active_stderr) {
        let fallback_output = std::process::Command::new(&program)
            .args(["auth", "status"])
            .output()
            .with_context(|| format!("failed to run `{program} auth status`"))?;
        if fallback_output.status.success() {
            return Ok(());
        }

        let fallback_stderr = String::from_utf8_lossy(&fallback_output.stderr)
            .trim()
            .to_string();
        let detail = if fallback_stderr.is_empty() {
            "authentication is unavailable".to_string()
        } else {
            fallback_stderr
        };
        anyhow::bail!(
            "GitHub branch-fix tasks require authenticated gh CLI access; `gh auth status` failed: {}",
            detail
        );
    }

    let detail = if active_stderr.is_empty() {
        "authentication is unavailable".to_string()
    } else {
        active_stderr
    };
    anyhow::bail!(
        "GitHub branch-fix tasks require authenticated gh CLI access; `gh auth status --active` failed: {}",
        detail
    );
}
fn gh_auth_status_active_unsupported(stderr: &str) -> bool {
    let trimmed = stderr.trim().to_ascii_lowercase();
    trimmed.contains("unknown flag: --active") || trimmed.contains("unknown flag `active`")
}
pub fn check_execution_rail(
    run: &CoordinationRun,
    task: &CoordinationTask,
    materialized_files: &[MaterializedArtifactFile],
) -> RailDecision {
    if materialized_files.is_empty() {
        return RailDecision::Allow;
    }
    if task.write_scope.is_empty() {
        return RailDecision::Reject {
            reason: "task captured materialized files without a declared write_scope".to_string(),
        };
    }
    if task_mutates_workspace(task) && !has_edit_tool(task) {
        return RailDecision::Reject {
            reason: "task captured workspace changes without declaring the edit tool".to_string(),
        };
    }
    for file in materialized_files {
        if task
            .write_scope
            .iter()
            .any(|scope| scope_allows_relative_path(scope, &file.path))
        {
            continue;
        }
        return RailDecision::Reject {
            reason: format!(
                "materialized file '{}' is outside declared write scope {:?}",
                file.path, task.write_scope
            ),
        };
    }
    if task.kind == CoordinationTaskKind::ApplyApprovedEdits
        && run.require_review_before_apply
        && !resolved_review_approval_exists(&run.run_id, task)
    {
        return RailDecision::Reject {
            reason: "apply task cannot sync back changes before review approval is resolved"
                .to_string(),
        };
    }
    RailDecision::Allow
}
pub fn check_output_rail(
    run: &CoordinationRun,
    task: &CoordinationTask,
    artifact: &TaskArtifact,
    materialized_files: &[MaterializedArtifactFile],
) -> RailDecision {
    if task_mutates_workspace(task) {
        let changed_paths: Vec<&str> = if lightweight_task_omits_changed_files(run, task) {
            materialized_files
                .iter()
                .map(|file| file.path.as_str())
                .collect()
        } else {
            artifact
                .report
                .changed_files
                .iter()
                .map(String::as_str)
                .collect()
        };
        let materialization_root = task_runsc_materialization_root(task);
        let skip_stub_marker_scan = task.kind == CoordinationTaskKind::MaterializeDesignDoc
            && run.workflow_mode == AutonomousWorkflowMode::CodeReview;
        for changed_file in changed_paths {
            if !skip_stub_marker_scan
                && file_contains_stub_markers(&materialization_root, changed_file).unwrap_or(false)
            {
                return RailDecision::Reject {
                    reason: format!("changed file '{changed_file}' contains stub markers"),
                };
            }
        }
    }
    RailDecision::Allow
}
