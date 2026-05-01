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
#[path = "claims_completion.rs"]
mod claims_completion;
#[path = "claims_scan.rs"]
mod claims_scan;
use super::*;
use claims_scan::load_claim_from_entry;
pub use claims_scan::{
    collect_claim_worker_pids_for_stop, count_active_claims, has_conflicting_write_scope,
    has_conflicting_write_scope_batched, reap_stale_claim_files,
};
pub fn write_presence(input: PresenceInput) -> Result<AgentPresence> {
    fs::create_dir_all(presence_dir())?;
    let presence = AgentPresence {
        provider: input.provider,
        instance_number: input.instance_number,
        instance_pid: input.instance_pid,
        current_load: input.current_load,
        visible_workspaces: input.visible_workspaces,
        last_heartbeat_at: Utc::now(),
    };
    write_json_atomic(&presence_path(presence.instance_pid), &presence)?;
    Ok(presence)
}
fn snapshot_lightweight_task_doc_before_write(
    _run_id: &str,
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> Result<()> {
    if !uses_lightweight_runtime_mode(&run.workflow_mode)
        || task.kind != CoordinationTaskKind::GeneratedWriteTask
    {
        return Ok(());
    }
    let _ = task;
    // Generated lightweight workers no longer own task-list edits, so there is
    // no task-doc snapshot rail to maintain here.
    Ok(())
}
pub fn persist_design_consensus_draft_doc_copy(
    task: &CoordinationTask,
    artifact: &TaskArtifact,
) -> Result<()> {
    if task.kind != CoordinationTaskKind::DraftDesignDoc {
        return Ok(());
    }

    let Some(provider) = task.assigned_provider.as_deref() else {
        return Ok(());
    };
    let run = load_run(&task.run_id)?;
    if run.workflow_mode != AutonomousWorkflowMode::DesignConsensus || !run.allow_write_tasks {
        return Ok(());
    }
    if artifact.body.trim().is_empty() {
        anyhow::bail!(
            "draft design artifact for task {} did not contain markdown to persist",
            task.task_id
        );
    }

    let draft_doc_path = provider_draft_design_doc_path(&run, provider);
    let absolute_path = workspace_path_for_run(&run, &draft_doc_path);
    if let Some(parent) = absolute_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&absolute_path, &artifact.body).with_context(|| {
        format!(
            "failed to write provider draft design doc extracted from task artifact to `{}`",
            draft_doc_path
        )
    })?;
    Ok(())
}
pub(crate) fn sync_run_summary_after_task_update(
    run_id: &str,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    let mut run = load_run(run_id)?;
    let tasks = load_tasks(run_id)?;
    let completion_gate_issues = if tasks
        .iter()
        .all(|task| task.status == CoordinationTaskStatus::Completed)
    {
        run_completion_gate_issues(&run, &tasks)?
    } else {
        Vec::new()
    };
    run.status = merge_derived_run_status(
        &run.status,
        derive_run_status(&tasks, &completion_gate_issues),
    );
    run.updated_at = updated_at;
    write_json_atomic(&run_path(run_id), &run)?;
    index_run(&run);
    refresh_lightweight_status_projection(run_id)?;
    if run.status == CoordinationRunStatus::Failed {
        match maybe_auto_fix_failed_run(run_id, &mut run) {
            Ok(true) => {
                let _ = restart_failed_tasks_with_mode(run_id, RestartMode::AutoSelfHeal)?;
            }
            Ok(false) => {}
            Err(err) => {
                warn!(
                    run_id = %run.run_id,
                    error = %err,
                    "automatic self-heal failed while syncing run summary"
                );
                let _ = append_event(
                    run_id,
                    "run_auto_fix_restart_failed",
                    None,
                    &format!("Automatic self-heal failed before restart: {err}"),
                );
            }
        }
    }
    Ok(())
}
pub fn lightweight_provider_unavailable_message(
    provider_name: &str,
    cooldown: Option<&ProviderCooldown>,
    detail: &str,
) -> String {
    let detail = detail.trim();
    match cooldown {
        Some(cooldown) if detail.is_empty() => format!(
            "[provider_unavailable] Configured lightweight provider {} is unavailable until {}. Lightweight tasks do not fall back to other providers. {}",
            provider_name,
            cooldown.unavailable_until.to_rfc3339(),
            cooldown.reason
        ),
        Some(cooldown) => format!(
            "[provider_unavailable] Configured lightweight provider {} is unavailable until {}. Lightweight tasks do not fall back to other providers. {} {}",
            provider_name,
            cooldown.unavailable_until.to_rfc3339(),
            cooldown.reason,
            detail
        ),
        None if detail.is_empty() => format!(
            "[provider_unavailable] Configured lightweight provider {} is unavailable. Lightweight tasks do not fall back to other providers.",
            provider_name
        ),
        None => format!(
            "[provider_unavailable] Configured lightweight provider {} is unavailable. Lightweight tasks do not fall back to other providers. {}",
            provider_name,
            detail
        ),
    }
}
pub fn infer_failure_kind(message: &str) -> TaskFailureKind {
    let trimmed = message.trim();
    if trimmed.contains("[provider_unavailable]") {
        TaskFailureKind::ProviderUnavailable
    } else if trimmed.contains("[git_publish_failed]") {
        TaskFailureKind::Publish
    } else if trimmed.contains("[blocked]") {
        TaskFailureKind::BlockedWrite
    } else if trimmed.contains("claim") && trimmed.contains("missing") {
        TaskFailureKind::ClaimLost
    } else if trimmed.contains("timeout") {
        TaskFailureKind::ProviderTimeout
    } else if trimmed.contains("acceptance") || trimmed.contains("validation") {
        TaskFailureKind::Acceptance
    } else {
        TaskFailureKind::Unknown
    }
}
pub fn recovery_action_for_failure_kind(kind: TaskFailureKind) -> TaskRecoveryAction {
    match kind {
        TaskFailureKind::Publish => TaskRecoveryAction::RetryPublish,
        TaskFailureKind::DependencyDrift
        | TaskFailureKind::MissingArtifact
        | TaskFailureKind::ClaimLost => TaskRecoveryAction::ReconcileState,
        TaskFailureKind::ProviderUnavailable
        | TaskFailureKind::ProviderTimeout
        | TaskFailureKind::Acceptance
        | TaskFailureKind::BlockedWrite
        | TaskFailureKind::WorkspaceMutation
        | TaskFailureKind::Unknown => TaskRecoveryAction::RetryTask,
    }
}
const SEMANTIC_REPAIR_CONTEXT_MARKER: &str = "Automatic repair context:";
pub(super) const AUTO_REPAIR_MAX_ATTEMPTS: u32 = 10;
pub(crate) fn is_pre_execution_rail_failure(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.starts_with("input rail rejected task:")
        || trimmed.starts_with("input rail interrupted task:")
        || trimmed.starts_with("input rail requested reask:")
        || trimmed.starts_with("dialog rail rejected task:")
        || trimmed.starts_with("dialog rail interrupted task:")
        || trimmed.starts_with("dialog rail requested reask:")
}
fn semantic_repair_prompt(prompt: &str, failure_message: &str) -> String {
    let base = prompt
        .split_once(SEMANTIC_REPAIR_CONTEXT_MARKER)
        .map(|(prefix, _)| prefix.trim_end())
        .unwrap_or_else(|| prompt.trim_end());
    format!(
        "{base}\n\n{marker}\n- The previous attempt did not satisfy the task contract or local checks.\n- Start from the current workspace state instead of redoing the task blindly.\n- Inspect the failure details below, fix the root cause, and rerun the most relevant local validation before reporting completion.\n- Keep the change minimal and scoped to this task.\nLatest failure:\n{failure_message}",
        marker = SEMANTIC_REPAIR_CONTEXT_MARKER,
    )
}
fn requeue_task_for_semantic_repair(
    run_id: &str,
    task: &mut CoordinationTask,
    provider_name: &str,
    failure_kind: TaskFailureKind,
    message: &str,
) -> Result<bool> {
    if !load_run(run_id)?.auto_retry_failed_tasks {
        return Ok(false);
    }
    if is_pre_execution_rail_failure(message) {
        return Ok(false);
    }
    if !matches!(
        failure_kind,
        TaskFailureKind::Acceptance
            | TaskFailureKind::Unknown
            | TaskFailureKind::BlockedWrite
            | TaskFailureKind::WorkspaceMutation
    ) {
        return Ok(false);
    }
    if !is_provider_backed(task) {
        return Ok(false);
    }
    if task.attempt_count >= task.max_attempts.max(1) {
        return Ok(false);
    }
    let now = Utc::now();
    if task.max_attempts < AUTO_REPAIR_MAX_ATTEMPTS {
        task.max_attempts = (task.max_attempts + 1).min(AUTO_REPAIR_MAX_ATTEMPTS);
    }
    task.prompt = semantic_repair_prompt(&task.prompt, message);
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.failure_kind = None;
    task.recovery_action = None;
    task.artifact_summary = Some(message.to_string());
    task.updated_at = now;
    write_json_atomic(&task_path(run_id, &task.task_id), task)?;
    index_task(task);
    let _ = fs::remove_file(claim_path(run_id, &task.task_id));
    append_event_with_provider(
        run_id,
        "task_repair_scheduled",
        Some(&task.task_id),
        Some(provider_name),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Scheduled automatic repair retry for {} after semantic failure. {}",
            task.title, message
        ),
    )?;
    sync_run_summary_after_task_update(run_id, now)?;
    Ok(true)
}
fn format_failure_attempt_suffix(current_attempt: u32, max_attempts: u32) -> String {
    if current_attempt <= 1 || max_attempts <= 1 {
        String::new()
    } else {
        format!(" after attempt {current_attempt}/{max_attempts}")
    }
}
pub fn adopt_claim_in_memory(
    claim: &mut TaskClaim,
    task: &mut CoordinationTask,
    launcher_pid: u32,
    worker_pid: u32,
    now: DateTime<Utc>,
) -> Result<()> {
    if claim.instance_pid != launcher_pid {
        anyhow::bail!(
            "task {} for run {} is owned by PID {}, not launcher {}",
            claim.task_id,
            claim.run_id,
            claim.instance_pid,
            launcher_pid
        );
    }

    claim.launcher_pid = Some(launcher_pid);
    claim.lifecycle_phase = ClaimLifecyclePhase::WorkerAdopted;
    claim.instance_pid = worker_pid;
    claim.worker_pid = Some(worker_pid);
    claim.worker_adopted_at = Some(now);
    claim.last_heartbeat_at = now;
    claim.worker_exited_at = None;
    claim.worker_exit_detail = None;

    task.claimed_by = Some(worker_pid);
    task.updated_at = now;
    Ok(())
}
pub fn adopt_claim_for_worker(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    worker_pid: u32,
) -> Result<()> {
    let now = Utc::now();
    let mut claim = load_claim(run_id, task_id)?;
    let mut task = load_task(run_id, task_id)?;
    adopt_claim_in_memory(&mut claim, &mut task, launcher_pid, worker_pid, now)?;
    write_json_atomic(&claim_path(run_id, task_id), &claim)?;
    write_json_atomic(&task_path(run_id, task_id), &task)?;
    Ok(())
}
pub fn expire_stale_claims(run_id: &str) -> Result<()> {
    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };
    let now = Utc::now();
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        if claim_heartbeat_stale_at(&claim) >= now {
            continue;
        }

        let mut task = load_task(run_id, &claim.task_id)?;
        if matches!(
            task.status,
            CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
        ) {
            task.attempt_count = task.attempt_count.saturating_sub(1);
            task.status = stale_claim_recovery_status(&task);
            task.claimed_by = None;
            task.artifact_summary = None;
            task.failure_kind = Some(TaskFailureKind::ClaimLost);
            task.recovery_action = Some(TaskRecoveryAction::ReconcileState);
            task.updated_at = now;
            write_json_atomic(&task_path(run_id, &task.task_id), &task)?;
            index_task(&task);
            append_event(
                run_id,
                "task_requeued",
                Some(&claim.task_id),
                &format!(
                    "Claim heartbeat went stale for instance PID {}. Claim progress: {}",
                    claim.instance_pid,
                    claim_lifecycle_summary(&claim)
                ),
            )?;
            sync_run_summary_after_task_update(run_id, task.updated_at)?;
        }
        let _ = fs::remove_file(entry.path());
    }
    Ok(())
}
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
pub fn stop_detached_workers(worker_pids: &[u32]) {
    for pid in worker_pids {
        #[cfg(unix)]
        unsafe {
            libc::kill(*pid as libc::pid_t, libc::SIGTERM);
        }
    }
    #[cfg(unix)]
    {
        for _ in 0..10 {
            if worker_pids.iter().all(|pid| !process_is_alive(*pid)) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        for pid in worker_pids {
            if process_is_alive(*pid) {
                unsafe {
                    libc::kill(*pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
    }
}
pub fn merge_retry_scope_entries(write_scope: &mut Vec<String>, additions: Vec<String>) -> bool {
    let mut changed = false;
    for addition in additions {
        if write_scope.contains(&addition) {
            continue;
        }
        write_scope.push(addition);
        changed = true;
    }
    changed
}
