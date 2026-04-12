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
