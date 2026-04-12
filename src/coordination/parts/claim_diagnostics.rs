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
