use super::*;

pub fn requeue_task_for_provider_cooldown(
    run_id: &str,
    task_id: &str,
    instance_pid: u32,
    provider: &str,
    message: &str,
) -> Result<bool> {
    let now = Utc::now();
    let normalized_provider =
        normalize_provider_name(Some(provider)).unwrap_or_else(|| provider.to_string());
    let Some(cooldown) = extract_provider_cooldown(&normalized_provider, message, now) else {
        return Ok(false);
    };

    let run = load_run(run_id)?;
    if !matches!(run.status, CoordinationRunStatus::Active) {
        return Ok(false);
    }

    let claim = match load_claim(run_id, task_id) {
        Ok(c) => c,
        Err(_) => return Ok(false),
    };
    if claim.instance_pid != instance_pid {
        return Ok(false);
    }

    let mut run = run;
    upsert_provider_cooldown(&mut run.provider_cooldowns, &normalized_provider, &cooldown);
    run.updated_at = now;
    write_json_atomic(&run_path(run_id), &run)?;

    let mut global_provider_cooldowns = load_global_provider_cooldowns()?;
    let global_changed = prune_expired_provider_cooldown_map(&mut global_provider_cooldowns, now)
        || upsert_provider_cooldown(
            &mut global_provider_cooldowns,
            &normalized_provider,
            &cooldown,
        );
    if global_changed {
        write_global_provider_cooldowns(&global_provider_cooldowns)?;
    }

    let mut task = load_task(run_id, task_id)?;
    if LightweightWorkflowBehavior::for_run(&run).is_some() && task.assigned_provider.is_some() {
        fail_lightweight_task_for_unavailable_provider(
            run_id,
            &mut task,
            &normalized_provider,
            Some(&cooldown),
            message,
        )?;
        return Ok(true);
    }
    let fallback_provider = select_fallback_provider(&run, &normalized_provider, now);
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.artifact_summary = Some(message.to_string());
    task.failure_kind = Some(TaskFailureKind::ProviderUnavailable);
    task.recovery_action = Some(TaskRecoveryAction::RetryTask);
    task.attempt_count = task.attempt_count.saturating_sub(1);
    task.updated_at = now;
    if let Some(fallback_provider) = fallback_provider.as_ref() {
        task.assigned_provider = Some(fallback_provider.clone());
    }
    write_json_atomic(&task_path(run_id, task_id), &task)?;

    let _ = fs::remove_file(claim_path(run_id, task_id));

    append_event_with_provider(
        run_id,
        "provider_cooldown",
        Some(task_id),
        Some(&normalized_provider),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Provider {} unavailable until {}. {}",
            normalized_provider,
            cooldown.unavailable_until.to_rfc3339(),
            cooldown.reason
        ),
    )?;
    append_event_with_provider(
        run_id,
        "task_retry_scheduled",
        Some(task_id),
        Some(&normalized_provider),
        task.task_category,
        task.provider_hint.as_deref(),
        &match fallback_provider {
            Some(fallback_provider) => format!(
                "Provider {} unavailable until {}. Requeued {} via {}. {}",
                normalized_provider,
                cooldown.unavailable_until.to_rfc3339(),
                task.title,
                fallback_provider,
                message
            ),
            None => format!(
                "Provider {} unavailable until {}. Requeued {} and will wait for provider recovery. {}",
                normalized_provider,
                cooldown.unavailable_until.to_rfc3339(),
                task.title,
                message
            ),
        },
    )?;
    Ok(true)
}

pub fn requeue_task_for_provider_timeout(
    run_id: &str,
    task_id: &str,
    instance_pid: u32,
    provider: &str,
    message: &str,
) -> Result<bool> {
    let normalized_provider =
        normalize_provider_name(Some(provider)).unwrap_or_else(|| provider.to_string());
    if !message
        .trim()
        .starts_with(&provider_timeout_prefix(&normalized_provider))
    {
        return Ok(false);
    }

    let run = load_run(run_id)?;
    let mut task = load_task(run_id, task_id)?;
    if !github_branch_fix_poll_task_should_respawn(&run, &task) {
        return Ok(false);
    }

    let claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(false),
    };
    if claim.instance_pid != instance_pid {
        return Ok(false);
    }

    let now = Utc::now();
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.artifact_summary = Some(message.to_string());
    task.failure_kind = Some(TaskFailureKind::ProviderTimeout);
    task.recovery_action = Some(TaskRecoveryAction::RetryTask);
    task.attempt_count = task.attempt_count.saturating_sub(1);
    task.updated_at = now;
    write_json_atomic(&task_path(run_id, task_id), &task)?;
    index_task(&task);
    let _ = fs::remove_file(claim_path(run_id, task_id));
    append_event_with_provider(
        run_id,
        "task_retry_scheduled",
        Some(task_id),
        Some(&normalized_provider),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Retry scheduled: {} after provider timeout. Returned the branch-fix poll task to queue without consuming attempt budget. {}",
            task.title, message
        ),
    )?;
    sync_run_summary_after_task_update(run_id, now)?;
    Ok(true)
}
