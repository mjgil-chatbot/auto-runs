use super::*;
use fs2::FileExt;
pub(crate) struct ClaimRefreshLock(fs::File);
impl Drop for ClaimRefreshLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn claim_refresh_lock_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("claim-refresh.lock")
}
pub(crate) fn acquire_claim_refresh_lock(run_id: &str) -> Result<ClaimRefreshLock> {
    let path = claim_refresh_lock_path(run_id);
    let parent = path
        .parent()
        .context("claim refresh lock path missing parent")?;
    fs::create_dir_all(parent)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    Ok(ClaimRefreshLock(file))
}
fn load_claims(run_id: &str) -> Vec<TaskClaim> {
    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    entries
        .flatten()
        .filter(|entry| entry.file_type().map(|ft| ft.is_file()).unwrap_or(false))
        .filter_map(|entry| fs::read_to_string(entry.path()).ok())
        .filter_map(|raw| serde_json::from_str::<TaskClaim>(&raw).ok())
        .collect()
}
fn missing_claim_details(task: &CoordinationTask) -> String {
    let owner = task
        .claimed_by
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    format!(
        "previous_owner_pid={owner}, attempt={}/{}",
        task.attempt_count.max(1),
        task.max_attempts.max(1)
    )
}
fn active_task_claim_requires_reconciliation(task: &CoordinationTask, claim: &TaskClaim) -> bool {
    task.claimed_by != Some(claim.instance_pid)
}
fn load_current_active_task_for_claim_repair(
    run_id: &str,
    task_id: &str,
) -> Result<Option<CoordinationTask>> {
    let task = load_task(run_id, task_id)?;
    Ok(matches!(
        task.status,
        CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
    )
    .then_some(task))
}
#[cfg(unix)]
fn process_is_alive_for_claim_repair(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}
#[cfg(not(unix))]
fn process_is_alive_for_claim_repair(_pid: u32) -> bool {
    true
}
fn should_defer_missing_claim_repair(task: &CoordinationTask) -> bool {
    let Some(owner_pid) = task.claimed_by else {
        return false;
    };
    process_is_alive_for_claim_repair(owner_pid)
}
pub(crate) fn reconcile_missing_claim_for_active_task_from_snapshot(
    run_id: &str,
    task: &CoordinationTask,
    now: DateTime<Utc>,
) -> Result<bool> {
    let Some(mut current_task) = load_current_active_task_for_claim_repair(run_id, &task.task_id)?
    else {
        return Ok(false);
    };
    if load_claim(run_id, &task.task_id).is_ok() {
        return Ok(false);
    }
    if should_defer_missing_claim_repair(&current_task) {
        return Ok(false);
    }

    let details = missing_claim_details(&current_task);
    let provider = current_task.assigned_provider.clone();
    append_event_with_provider(
        run_id,
        "task_claim_missing",
        Some(&current_task.task_id),
        provider.as_deref(),
        current_task.task_category,
        current_task.provider_hint.as_deref(),
        &with_driver_context(
            format!(
                "Detected missing claim file for active task {}. {}.",
                current_task.title, details
            ),
            std::process::id(),
        ),
    )?;

    current_task.attempt_count = current_task.attempt_count.saturating_sub(1);
    current_task.status = stale_claim_recovery_status(&current_task);
    current_task.claimed_by = None;
    current_task.artifact_summary = None;
    current_task.failure_kind = Some(TaskFailureKind::ClaimLost);
    current_task.recovery_action = Some(TaskRecoveryAction::ReconcileState);
    current_task.updated_at = now;
    write_json_atomic(&task_path(run_id, &current_task.task_id), &current_task)?;
    index_task(&current_task);

    append_event_with_provider(
        run_id,
        "task_requeued",
        Some(&current_task.task_id),
        provider.as_deref(),
        current_task.task_category,
        current_task.provider_hint.as_deref(),
        &with_driver_context(
            format!(
                "Recovered active task {} after missing claim detection. {}.",
                current_task.title, details
            ),
            std::process::id(),
        ),
    )?;
    Ok(true)
}
pub fn reconcile_orphaned_claim_files(run_id: &str) -> Result<bool> {
    let mut changed = false;

    for claim in load_claims(run_id) {
        let current_task = match load_task(run_id, &claim.task_id) {
            Ok(task) => task,
            Err(_) => {
                let _ = fs::remove_file(claim_path(run_id, &claim.task_id));
                append_event(
                    run_id,
                    "task_claim_orphaned",
                    Some(&claim.task_id),
                    &with_driver_context(
                        format!(
                            "Removed orphan claim file for missing task {} owned by PID {}.",
                            claim.task_id, claim.instance_pid
                        ),
                        std::process::id(),
                    ),
                )?;
                changed = true;
                continue;
            }
        };

        if matches!(
            current_task.status,
            CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
        ) {
            continue;
        }

        let status = current_task.status;
        if current_task.claimed_by == Some(claim.instance_pid)
            && should_defer_missing_claim_repair(&current_task)
        {
            continue;
        }

        if load_claim(run_id, &claim.task_id)
            .map(|latest_claim| latest_claim.instance_pid != claim.instance_pid)
            .unwrap_or(true)
        {
            continue;
        }

        let _ = fs::remove_file(claim_path(run_id, &claim.task_id));
        append_event(
            run_id,
            "task_claim_orphaned",
            Some(&claim.task_id),
            &with_driver_context(
                format!(
                    "Removed orphan claim file for task {} because durable task status is {:?}, not running/claiming.",
                    claim.task_id, status
                ),
                std::process::id(),
            ),
        )?;
        changed = true;
    }

    Ok(changed)
}
