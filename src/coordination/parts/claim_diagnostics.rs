use super::*;
use std::process::ExitStatus;

fn append_claim_event(run_id: &str, task_id: &str, event_type: &str, message: &str) -> Result<()> {
    match load_task(run_id, task_id) {
        Ok(task) => append_event_with_provider(
            run_id,
            event_type,
            Some(task_id),
            task.assigned_provider.as_deref(),
            task.task_category,
            task.provider_hint.as_deref(),
            message,
        ),
        Err(_) => append_event(run_id, event_type, Some(task_id), message),
    }
}

fn append_claim_event_with_provider(
    run_id: &str,
    task_id: &str,
    provider: &str,
    event_type: &str,
    message: &str,
) -> Result<()> {
    match load_task(run_id, task_id) {
        Ok(task) => append_event_with_provider(
            run_id,
            event_type,
            Some(task_id),
            Some(provider),
            task.task_category,
            task.provider_hint.as_deref(),
            message,
        ),
        Err(_) => append_event_with_provider(
            run_id,
            event_type,
            Some(task_id),
            Some(provider),
            None,
            None,
            message,
        ),
    }
}

fn timestamp_field(label: &str, value: Option<DateTime<Utc>>) -> Option<String> {
    value.map(|at| format!("{label}={}", at.to_rfc3339()))
}

pub fn with_driver_context(message: String, instance_pid: u32) -> String {
    match crate::driver::current_driver_claim_context(instance_pid) {
        Some(context) => format!("{message} [{context}]"),
        None => message,
    }
}

pub fn claim_lifecycle_summary(claim: &TaskClaim) -> String {
    let mut fields = vec![
        format!("phase={}", claim.lifecycle_phase.as_str()),
        format!("owner_pid={}", claim.instance_pid),
    ];
    if let Some(pid) = claim.launcher_pid {
        fields.push(format!("launcher_pid={pid}"));
    }
    if let Some(pid) = claim.worker_pid {
        fields.push(format!("worker_pid={pid}"));
    }
    fields.extend(
        [
            timestamp_field("claimed_at", Some(claim.claimed_at)),
            timestamp_field("worker_spawned_at", claim.worker_spawned_at),
            timestamp_field("worker_adopted_at", claim.worker_adopted_at),
            timestamp_field("execution_started_at", claim.execution_started_at),
            timestamp_field("last_heartbeat_at", Some(claim.last_heartbeat_at)),
            timestamp_field("stale_after_at", Some(claim_heartbeat_stale_at(claim))),
            timestamp_field("worker_exited_at", claim.worker_exited_at),
        ]
        .into_iter()
        .flatten(),
    );
    if let Some(detail) = claim.worker_exit_detail.as_deref() {
        fields.push(format!("worker_exit_detail={detail}"));
    }
    fields.join(", ")
}

pub fn record_worker_spawned(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    worker_pid: u32,
) -> Result<()> {
    let mut claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(()),
    };
    if claim.instance_pid != launcher_pid {
        return Ok(());
    }

    let now = Utc::now();
    claim.lifecycle_phase = ClaimLifecyclePhase::WorkerSpawned;
    claim.worker_pid = Some(worker_pid);
    claim.worker_spawned_at = Some(now);
    claim.worker_exited_at = None;
    claim.worker_exit_detail = None;
    write_json_atomic(&claim_path(run_id, task_id), &claim)?;

    append_claim_event(
        run_id,
        task_id,
        "task_worker_spawned",
        &format!(
            "Spawned detached worker PID {worker_pid}. Claim progress: {}.",
            claim_lifecycle_summary(&claim)
        ),
    )
}

pub fn record_worker_started(
    run_id: &str,
    task_id: &str,
    provider: &str,
    launcher_pid: u32,
    worker_pid: u32,
) -> Result<()> {
    append_claim_event_with_provider(
        run_id,
        task_id,
        provider,
        "task_worker_started",
        &format!(
            "Worker PID {worker_pid} started for task {task_id} with launcher PID {launcher_pid}."
        ),
    )
}

pub fn record_worker_adopted(run_id: &str, task_id: &str, worker_pid: u32) -> Result<()> {
    let claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(()),
    };
    append_claim_event(
        run_id,
        task_id,
        "task_worker_adopted",
        &format!(
            "Worker PID {worker_pid} adopted the claim. Claim progress: {}.",
            claim_lifecycle_summary(&claim)
        ),
    )
}

pub fn record_task_execution_started(run_id: &str, task_id: &str, instance_pid: u32) -> Result<()> {
    let mut claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(()),
    };
    if claim.instance_pid != instance_pid {
        return Ok(());
    }

    let now = Utc::now();
    claim.lifecycle_phase = ClaimLifecyclePhase::TaskExecutionStarted;
    claim.worker_pid.get_or_insert(instance_pid);
    claim.worker_adopted_at.get_or_insert(now);
    claim.execution_started_at = Some(now);
    claim.last_heartbeat_at = now;
    claim.worker_exited_at = None;
    claim.worker_exit_detail = None;
    write_json_atomic(&claim_path(run_id, task_id), &claim)?;

    append_claim_event(
        run_id,
        task_id,
        "task_execution_started",
        &format!(
            "Worker PID {instance_pid} began task execution. Claim progress: {}.",
            claim_lifecycle_summary(&claim)
        ),
    )
}

pub fn record_worker_adopt_failed(
    run_id: &str,
    task_id: &str,
    provider: &str,
    launcher_pid: u32,
    worker_pid: u32,
    detail: &str,
) -> Result<()> {
    let claim_summary = load_claim(run_id, task_id)
        .map(|claim| claim_lifecycle_summary(&claim))
        .unwrap_or_else(|_| "claim_missing".to_string());
    append_claim_event_with_provider(
        run_id,
        task_id,
        provider,
        "task_worker_adopt_failed",
        &format!(
            "Worker PID {worker_pid} failed to adopt the claim from launcher PID {launcher_pid}. {detail}. Claim progress: {claim_summary}."
        ),
    )
}

pub fn record_worker_exited(
    run_id: &str,
    task_id: &str,
    worker_pid: u32,
    detail: &str,
) -> Result<()> {
    let mut claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(()),
    };
    if claim.worker_pid != Some(worker_pid) && claim.instance_pid != worker_pid {
        return Ok(());
    }

    let now = Utc::now();
    claim.lifecycle_phase = ClaimLifecyclePhase::WorkerExited;
    claim.worker_pid.get_or_insert(worker_pid);
    claim.worker_exited_at = Some(now);
    claim.worker_exit_detail = Some(detail.to_string());
    write_json_atomic(&claim_path(run_id, task_id), &claim)?;

    append_claim_event(
        run_id,
        task_id,
        "task_worker_exited",
        &format!(
            "Worker PID {worker_pid} exited before the task reached a terminal artifact. {detail}. Claim progress: {}.",
            claim_lifecycle_summary(&claim)
        ),
    )
}

#[cfg(unix)]
pub fn format_worker_exit_status(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;

    if let Some(code) = status.code() {
        return format!("exit code {code}");
    }
    if let Some(signal) = status.signal() {
        return format!("signal {signal}");
    }
    status.to_string()
}

#[cfg(not(unix))]
pub fn format_worker_exit_status(status: ExitStatus) -> String {
    status
        .code()
        .map(|code| format!("exit code {code}"))
        .unwrap_or_else(|| status.to_string())
}
