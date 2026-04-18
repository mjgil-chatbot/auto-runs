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
