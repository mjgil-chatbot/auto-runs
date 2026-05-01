use super::*;

pub(super) fn load_claim_from_entry(entry: &fs::DirEntry) -> Option<TaskClaim> {
    let Ok(raw) = fs::read_to_string(entry.path()) else {
        return None;
    };
    serde_json::from_str(&raw).ok()
}

pub fn count_active_claims(run_id: &str) -> Result<usize> {
    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };

    let now = Utc::now();
    let mut count = 0;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        if claim_heartbeat_stale_at(&claim) >= now {
            count += 1;
        }
    }

    Ok(count)
}

/// Reap tasks whose claim files have gone stale (no heartbeat for
/// TASK_HEARTBEAT_STALE_SECS).  Unlike the council's in-memory `active_claims`
/// revocation (which only tracks claimed scopes and emits ledger entries), this
/// operates on the durable claim files and transitions the actual task status
/// back to `Queued` so it can be re-claimed.  Returns the count of reaped tasks.
pub fn reap_stale_claim_files(run_id: &str) -> Result<usize> {
    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };

    let now = Utc::now();
    let mut reaped = 0;

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        // Only reap claim files whose heartbeat is stale (no heartbeat for 5 min).
        if claim_heartbeat_stale_at(&claim) >= now {
            continue;
        }

        let task_id = &claim.task_id;
        let Ok(mut task) = load_task(run_id, task_id) else {
            // Task file gone — clean up orphaned claim file.
            let _ = fs::remove_file(entry.path());
            continue;
        };

        if !matches!(
            task.status,
            CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
        ) {
            let _ = fs::remove_file(entry.path());
            continue;
        }

        task.attempt_count = task.attempt_count.saturating_sub(1);
        task.status = CoordinationTaskStatus::Queued;
        task.claimed_by = None;
        task.artifact_summary =
            Some("Reaped after stale heartbeat — worker did not report progress.".to_string());
        task.failure_kind = Some(TaskFailureKind::ClaimLost);
        task.recovery_action = Some(TaskRecoveryAction::ReconcileState);
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(entry.path());

        append_event(
            run_id,
            "task_requeued",
            Some(task_id),
            &format!(
                "Reaped stale claim for '{}': worker PID {} did not send a heartbeat within {}s. Task returned to {} (attempt {}/{}).",
                task.title,
                claim.worker_pid.unwrap_or(claim.instance_pid),
                TASK_HEARTBEAT_STALE_SECS,
                "Queued",
                task.attempt_count,
                task.max_attempts.max(1),
            ),
        )?;
        sync_run_summary_after_task_update(run_id, task.updated_at)?;
        reaped += 1;
    }

    Ok(reaped)
}

fn claim_worker_pids_for_stop(claim: &TaskClaim) -> Vec<u32> {
    let mut pids = Vec::new();
    if let Some(worker_pid) = claim.worker_pid {
        pids.push(worker_pid);
    }
    if claim
        .launcher_pid
        .map(|launcher_pid| launcher_pid != claim.instance_pid)
        .unwrap_or(true)
        && !pids.contains(&claim.instance_pid)
    {
        pids.push(claim.instance_pid);
    }
    pids
}

pub fn collect_claim_worker_pids_for_stop(run_id: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return pids,
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        for worker_pid in claim_worker_pids_for_stop(&claim) {
            if !pids.contains(&worker_pid) {
                pids.push(worker_pid);
            }
        }
    }

    pids
}

pub fn has_conflicting_write_scope(
    run_id: &str,
    candidate_task_id: &str,
    candidate_scopes: &[String],
) -> Result<bool> {
    if candidate_scopes.is_empty() {
        return Ok(false);
    }

    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(false),
    };

    let now = Utc::now();
    let run = load_run(run_id)?;
    let candidate_task = load_task(run_id, candidate_task_id)?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        if claim_heartbeat_stale_at(&claim) < now || claim.task_id == candidate_task_id {
            continue;
        }
        let other_task = load_task(run_id, &claim.task_id)?;
        if other_task.write_scope.is_empty() {
            continue;
        }
        if lightweight_write_scope_conflict_is_coordination_only(
            &run,
            &candidate_task,
            candidate_scopes,
            &other_task,
        ) {
            continue;
        }
        if scopes_conflict(candidate_scopes, &other_task.write_scope) {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Batched version of `has_conflicting_write_scope` — accepts all tasks in memory
/// to avoid N per-claim task file reads during task claiming.
pub fn has_conflicting_write_scope_batched(
    run_id: &str,
    candidate_task_id: &str,
    candidate_scopes: &[String],
    tasks_by_id: &BTreeMap<String, CoordinationTask>,
) -> Result<bool> {
    if candidate_scopes.is_empty() {
        return Ok(false);
    }

    let entries = match fs::read_dir(claims_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(false),
    };

    let now = Utc::now();
    let run = load_run(run_id)?;
    let Some(candidate_task) = tasks_by_id.get(candidate_task_id) else {
        return Ok(false);
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let Some(claim) = load_claim_from_entry(&entry) else {
            continue;
        };
        if claim_heartbeat_stale_at(&claim) < now || claim.task_id == candidate_task_id {
            continue;
        }
        let Some(other_task) = tasks_by_id.get(&claim.task_id) else {
            continue;
        };
        if other_task.write_scope.is_empty() {
            continue;
        }
        if lightweight_write_scope_conflict_is_coordination_only(
            &run,
            candidate_task,
            candidate_scopes,
            other_task,
        ) {
            continue;
        }
        if scopes_conflict(candidate_scopes, &other_task.write_scope) {
            return Ok(true);
        }
    }

    Ok(false)
}
