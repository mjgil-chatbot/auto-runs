use super::*;
use fs2::FileExt;

struct ReconcileRunLock(fs::File);

impl Drop for ReconcileRunLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn reconcile_run_lock_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("reconcile.lock")
}

fn try_acquire_reconcile_run_lock(run_id: &str) -> Result<Option<ReconcileRunLock>> {
    let path = reconcile_run_lock_path(run_id);
    let parent = path
        .parent()
        .context("reconcile lock path missing parent")?;
    fs::create_dir_all(parent)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(ReconcileRunLock(file))),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to lock {}", path.display())),
    }
}

fn append_reconcile_issue(
    run_id: &str,
    task: Option<&CoordinationTask>,
    issue: &str,
    repaired: bool,
) -> Result<()> {
    let event_type = if repaired {
        "run_reconcile_issue_repaired"
    } else {
        "run_reconcile_issue_detected"
    };
    append_event_with_provider(
        run_id,
        event_type,
        task.map(|task| task.task_id.as_str()),
        task.and_then(|task| task.assigned_provider.as_deref()),
        task.and_then(|task| task.task_category),
        task.and_then(|task| task.provider_hint.as_deref()),
        issue,
    )
}

fn short_commit(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}

pub(crate) fn load_incomplete_mutation_batches(
    run_id: &str,
    processed: &BTreeSet<String>,
) -> Result<Vec<CoordinationMutationBatch>> {
    let entries = match fs::read_dir(mutation_batches_dir(run_id)) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    let now = Utc::now();
    let mut batches = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let raw = match fs::read_to_string(entry.path()) {
            Ok(raw) => raw,
            Err(_) => continue,
        };
        let Ok(batch) = serde_json::from_str::<CoordinationMutationBatch>(&raw) else {
            continue;
        };
        // Skip batches with future created_at (clock skew / corrupted state) — these
        // indicate a previous crash left the batch file in an inconsistent state and
        // should not block forward progress. A properly created batch will have
        // created_at <= now, so a batch with created_at > now is never valid.
        if batch.created_at > now {
            complete_mutation_batch(&mut batch.clone())?;
            append_event(
                run_id,
                "mutation_batch_replay_skipped",
                batch.task_id.as_deref(),
                &format!(
                    "Skipped mutation batch {:?} with future created_at {} — marked complete to unblock progress.",
                    batch.kind,
                    batch.created_at.to_rfc3339()
                ),
            )?;
            continue;
        }
        // Defensive: also skip if completed_at is in the future (shouldn't happen
        // with Utc::now() but guards against manual file tampering)
        if let Some(completed) = batch.completed_at {
            if completed > now {
                continue;
            }
        }
        if batch.completed_at.is_none() && !processed.contains(&batch.batch_id) {
            batches.push(batch);
        }
    }
    batches.sort_by(|left, right| left.created_at.cmp(&right.created_at));
    Ok(batches)
}

fn replay_complete_task_batch(batch: &CoordinationMutationBatch) -> Result<bool> {
    let Some(task_id) = batch.task_id.as_deref() else {
        return Ok(false);
    };
    let run = load_run(&batch.run_id)?;
    let mut task = load_task(&batch.run_id, task_id)?;
    if matches!(
        task.status,
        CoordinationTaskStatus::Completed
            | CoordinationTaskStatus::Failed
            | CoordinationTaskStatus::Canceled
    ) {
        return Ok(false);
    }
    let artifact_path = artifact_result_path(&batch.run_id, task_id);
    if !artifact_path.is_file() {
        return Ok(false);
    }
    let raw = fs::read_to_string(&artifact_path)?;
    let artifact: TaskArtifact = serde_json::from_str(&raw)?;
    let validation_failed = !artifact.report.validation_errors.is_empty();
    let explicit_failure = artifact.report.outcome == CoordinationTaskOutcome::Failed;
    let blocked_write_failure = task_mutates_workspace(&task)
        && artifact.report.outcome == CoordinationTaskOutcome::Blocked;
    let failure_message = if validation_failed && !blocked_write_failure {
        format!(
            "{} [{}]. Acceptance checks failed: {}",
            artifact.summary,
            artifact.report.outcome.as_str(),
            artifact.report.validation_errors.join(" | ")
        )
    } else {
        format!(
            "{} [{}]",
            artifact.summary,
            artifact.report.outcome.as_str()
        )
    };

    if validation_failed || explicit_failure || blocked_write_failure {
        let failure_kind = infer_failure_kind(&failure_message);
        let can_retry = should_auto_retry_task(&run, &task, Some(&failure_message));
        task.status = if can_retry {
            CoordinationTaskStatus::Queued
        } else {
            CoordinationTaskStatus::Failed
        };
        task.claimed_by = None;
        task.artifact_summary = Some(failure_message);
        task.failure_kind = Some(failure_kind);
        task.recovery_action = Some(if can_retry {
            recovery_action_for_failure_kind(failure_kind)
        } else {
            TaskRecoveryAction::ManualIntervention
        });
    } else {
        task.status = CoordinationTaskStatus::Completed;
        task.claimed_by = None;
        task.artifact_summary = Some(format!(
            "{} [{}]",
            artifact.summary,
            artifact.report.outcome.as_str()
        ));
        if uses_lightweight_runtime_mode(&run.workflow_mode)
            && task.kind == CoordinationTaskKind::GeneratedWriteTask
        {
            task.failure_kind = Some(TaskFailureKind::Publish);
            task.recovery_action = Some(TaskRecoveryAction::RetryPublish);
        } else {
            task.failure_kind = None;
            task.recovery_action = None;
        }
    }
    task.updated_at = Utc::now();
    write_json_atomic(&task_path(&batch.run_id, task_id), &task)?;
    index_task(&task);
    let _ = fs::remove_file(claim_path(&batch.run_id, task_id));
    sync_run_summary_after_task_update(&batch.run_id, task.updated_at)?;
    Ok(true)
}

pub fn replay_incomplete_mutation_batches_for_run(run_id: &str) -> Result<bool> {
    let mut run = load_run(run_id)?;
    let processed = run.processed_mutation_batch_ids.clone();
    let batches = load_incomplete_mutation_batches(run_id, &processed)?;
    if batches.is_empty() {
        return Ok(false);
    }
    let mut changed = false;
    let mut new_processed = processed;
    for mut batch in batches {
        if batch.kind == CoordinationMutationKind::ReconcileRun {
            complete_mutation_batch(&mut batch)?;
            new_processed.insert(batch.batch_id.clone());
            continue;
        }
        append_event(
            run_id,
            "mutation_batch_replay_detected",
            batch.task_id.as_deref(),
            &format!(
                "Replaying incomplete mutation batch {:?} created at {}.",
                batch.kind,
                batch.created_at.to_rfc3339()
            ),
        )?;
        match batch.kind {
            CoordinationMutationKind::CompleteTask => {
                changed |= replay_complete_task_batch(&batch)?;
            }
            CoordinationMutationKind::RestartFailedTasks => {
                complete_mutation_batch(&mut batch)?;
                let _ = restart_failed_tasks_with_mode_locked(run_id, RestartMode::Manual)?;
                changed = true;
                new_processed.insert(batch.batch_id.clone());
                continue;
            }
            CoordinationMutationKind::PublishWorkspaceDelta => {}
            CoordinationMutationKind::ReconcileRun => unreachable!(),
        }
        complete_mutation_batch(&mut batch)?;
        new_processed.insert(batch.batch_id.clone());
        append_event(
            run_id,
            "mutation_batch_replay_completed",
            batch.task_id.as_deref(),
            &format!("Replayed incomplete mutation batch {:?}.", batch.kind),
        )?;
    }
    if new_processed != run.processed_mutation_batch_ids {
        run.processed_mutation_batch_ids = new_processed;
        run.updated_at = Utc::now();
        write_json_atomic(&run_path(&run.run_id), &run)?;
    }
    Ok(changed)
}

fn reconcile_dependency_generations(run_id: &str, tasks: &mut [CoordinationTask]) -> Result<bool> {
    let completed = completed_task_generations(tasks);
    let mut changed = false;

    for task in tasks.iter_mut() {
        let Some(generations) = task
            .dependencies
            .iter()
            .map(|dependency| completed.get(dependency).copied())
            .collect::<Option<Vec<u32>>>()
        else {
            continue;
        };
        let Some(dependency_generation) = generations.into_iter().next() else {
            continue;
        };
        if dependency_generation == effective_dependency_generation(task) {
            continue;
        }
        append_reconcile_issue(
            run_id,
            Some(task),
            &format!(
                "Detected dependency generation drift for {}. task_generation={}, dependency_generation={}.",
                task.title,
                effective_dependency_generation(task),
                dependency_generation
            ),
            false,
        )?;
        task.dependency_generation = Some(dependency_generation);
        task.failure_kind = Some(TaskFailureKind::DependencyDrift);
        task.recovery_action = Some(TaskRecoveryAction::ReconcileState);
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        append_reconcile_issue(
            run_id,
            Some(task),
            &format!(
                "Rebased dependency generation for {} onto {}.",
                task.title, dependency_generation
            ),
            true,
        )?;
        changed = true;
    }

    if !unblock_runnable_task_ids(tasks).is_empty() {
        persist_selected_tasks(
            run_id,
            tasks,
            &tasks
                .iter()
                .map(|task| task.task_id.clone())
                .collect::<Vec<_>>(),
        )?;
        changed = true;
    }
    Ok(changed)
}

fn reconcile_completed_task_artifacts(
    run_id: &str,
    tasks: &mut [CoordinationTask],
) -> Result<bool> {
    let mut changed = false;
    for task in tasks.iter_mut() {
        if task.status != CoordinationTaskStatus::Completed {
            continue;
        }
        if artifact_result_path(run_id, &task.task_id).is_file() {
            if task.failure_kind == Some(TaskFailureKind::MissingArtifact)
                || task.recovery_action == Some(TaskRecoveryAction::ReconcileState)
            {
                task.failure_kind = None;
                task.recovery_action = None;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                index_task(task);
                changed = true;
            }
            continue;
        }
        if completed_task_has_acceptable_artifact_fallback(task) {
            if task.failure_kind == Some(TaskFailureKind::MissingArtifact)
                || task.recovery_action == Some(TaskRecoveryAction::ReconcileState)
            {
                append_reconcile_issue(
                    run_id,
                    Some(task),
                    &format!(
                        "Accepted persisted task summary as the artifact fallback for completed task {}.",
                        task.title
                    ),
                    true,
                )?;
                task.failure_kind = None;
                task.recovery_action = None;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                index_task(task);
                changed = true;
            }
            continue;
        }
        if task.failure_kind == Some(TaskFailureKind::MissingArtifact)
            && task.recovery_action == Some(TaskRecoveryAction::ReconcileState)
        {
            continue;
        }
        append_reconcile_issue(
            run_id,
            Some(task),
            &format!(
                "Completed task {} is missing its structured artifact.",
                task.title
            ),
            false,
        )?;
        task.failure_kind = Some(TaskFailureKind::MissingArtifact);
        task.recovery_action = Some(TaskRecoveryAction::ReconcileState);
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        changed = true;
    }
    Ok(changed)
}

fn ensure_publish_repair_task(
    run: &mut CoordinationRun,
    tasks: &mut Vec<CoordinationTask>,
    source_task: &CoordinationTask,
    error_message: &str,
) -> Result<bool> {
    if source_task.title.starts_with("Repair git publish for ") {
        return Ok(false);
    }
    let title = format!("Repair git publish for {}", source_task.title);
    if tasks.iter().any(|task| {
        task.parent_task_id.as_deref() == Some(source_task.task_id.as_str()) && task.title == title
    }) {
        return Ok(false);
    }

    let now = Utc::now();
    let task_id = Uuid::new_v4().to_string();
    let repair_task = CoordinationTask {
        task_id: task_id.clone(),
        run_id: run.run_id.clone(),
        parent_task_id: Some(source_task.task_id.clone()),
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: source_task.assigned_provider.clone(),
        task_category: source_task.task_category,
        provider_hint: source_task.provider_hint.clone(),
        required_role: source_task.required_role,
        iteration: None,
        title: title.clone(),
        prompt: format!(
            "The runtime could not finish automatic git publish for the completed task `{}`.\n\nStart from the current workspace state and fix the publish blocker with the smallest possible change.\n- Inspect git status, commit history, branch state, and remote state.\n- If code changes are already correct, avoid unnecessary edits and focus on the publish blocker.\n- Rerun the most relevant local checks before finishing.\n\nLatest publish failure:\n{}",
            source_task.title, error_message
        ),
        cwd: source_task.cwd.clone(),
        allowed_tools: source_task.allowed_tools.clone(),
        read_scope: source_task.read_scope.clone(),
        write_scope: source_task.write_scope.clone(),
        acceptance_checks: source_task.acceptance_checks.clone(),
        review_metadata: None,
        dependencies: vec![source_task.task_id.clone()],
        execution_generation: source_task.execution_generation,
        dependency_generation: Some(effective_dependency_generation(source_task)),
        attempt_count: 0,
        max_attempts: source_task.max_attempts.max(1),
        publish_attempt_count: 0,
        estimated_minutes: source_task.estimated_minutes,
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    run.task_ids.push(task_id.clone());
    run.updated_at = now;
    write_json_atomic(&run_path(&run.run_id), run)?;
    index_run(run);
    write_json_atomic(&task_path(&run.run_id, &task_id), &repair_task)?;
    index_task(&repair_task);
    tasks.push(repair_task);
    append_event_with_provider(
        &run.run_id,
        "task_repair_scheduled",
        Some(&task_id),
        source_task.assigned_provider.as_deref(),
        source_task.task_category,
        source_task.provider_hint.as_deref(),
        &format!(
            "Created publish repair task for {}. {}",
            source_task.title, error_message
        ),
    )?;
    Ok(true)
}

fn reconcile_deferred_lightweight_publish(
    run: &mut CoordinationRun,
    tasks: &mut Vec<CoordinationTask>,
    task_index: usize,
) -> Result<bool> {
    let mut publish_repair: Option<(CoordinationTask, String)> = None;
    {
        let task = &mut tasks[task_index];
        if !uses_lightweight_runtime_mode(&run.workflow_mode)
            || task.kind != CoordinationTaskKind::GeneratedWriteTask
            || task.recovery_action != Some(TaskRecoveryAction::RetryPublish)
        {
            return Ok(false);
        }

        let materialized_files = load_artifact_materialized_files(&run.run_id, &task.task_id);
        append_reconcile_issue(
            &run.run_id,
            Some(task),
            &format!(
                "Deferred git publish pending for {}. publish_attempt={}/{}.",
                task.title,
                task.publish_attempt_count.saturating_add(1),
                task.max_attempts.max(1)
            ),
            false,
        )?;

        match publish_lightweight_generated_write_task(task, &materialized_files) {
            Ok(commit) => {
                task.publish_attempt_count = task.publish_attempt_count.saturating_add(1);
                task.failure_kind = None;
                task.recovery_action = None;
                if LightweightWorkflowBehavior::for_run(run)
                    .is_some_and(|behavior| !behavior.persists_materialized_files())
                {
                    // The exact payload was needed while publication was pending.
                    // Compact lightweight state resumes after publication succeeds.
                    let _ = fs::remove_file(artifact_materialized_files_path(
                        &run.run_id,
                        &task.task_id,
                    ));
                }
                if let Some(commit) = commit.as_deref().map(short_commit) {
                    let summary = task
                        .artifact_summary
                        .clone()
                        .unwrap_or_else(|| task.title.clone());
                    task.artifact_summary =
                        Some(format!("{summary} Published git commit {commit}."));
                }
                write_json_atomic(&task_path(&run.run_id, &task.task_id), task)?;
                index_task(task);
                append_reconcile_issue(
                    &run.run_id,
                    Some(task),
                    &format!("Deferred git publish succeeded for {}.", task.title),
                    true,
                )?;
            }
            Err(err) => {
                task.publish_attempt_count = task.publish_attempt_count.saturating_add(1);
                if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
                    task.failure_kind = Some(TaskFailureKind::Publish);
                    task.recovery_action = None;
                    let provider = task
                        .assigned_provider
                        .clone()
                        .unwrap_or_else(|| run.provider.clone());
                    if requeue_github_branch_fix_publish_failure(
                        &run.run_id,
                        task,
                        &provider,
                        &format!("[git_publish_failed] {err}"),
                    )? {
                        return Ok(true);
                    }
                }
                task.failure_kind = Some(TaskFailureKind::Publish);
                task.artifact_summary = Some(format!("Deferred git publish failed: {err}"));
                if task.publish_attempt_count >= task.max_attempts.max(1) {
                    task.recovery_action = None;
                    publish_repair = Some((task.clone(), err.to_string()));
                } else {
                    task.recovery_action = Some(TaskRecoveryAction::RetryPublish);
                }
                write_json_atomic(&task_path(&run.run_id, &task.task_id), task)?;
                index_task(task);
                append_reconcile_issue(
                    &run.run_id,
                    Some(task),
                    &format!("Deferred git publish failed for {}: {}", task.title, err),
                    false,
                )?;
            }
        }
    }
    if let Some((task_snapshot, error_message)) = publish_repair {
        ensure_publish_repair_task(run, tasks, &task_snapshot, &error_message)?;
    }
    Ok(true)
}

pub fn reconcile_run_state(run_id: &str) -> Result<bool> {
    let Some(_lock) = try_acquire_reconcile_run_lock(run_id)? else {
        return Ok(false);
    };
    let replayed = replay_incomplete_mutation_batches_for_run(run_id)?;
    // Council workflows own a separate ledger, not the task-DAG lifecycle.
    if load_run(run_id)?.workflow_mode == AutonomousWorkflowMode::FreeformCouncil {
        return Ok(replayed);
    }
    let mut changed = replayed;
    changed |= reconcile_tasks_missing_claims(run_id)?;
    changed |= reconcile_orphaned_claim_files(run_id)?;

    let mut run = load_run(run_id)?;
    if run.status == CoordinationRunStatus::Canceled {
        if changed {
            run.updated_at = Utc::now();
            write_json_atomic(&run_path(&run.run_id), &run)?;
            index_run(&run);
        }
        return Ok(changed);
    }
    let mut tasks = load_tasks(run_id)?;
    changed |= reconcile_dependency_generations(run_id, &mut tasks)?;
    changed |= reconcile_completed_task_artifacts(run_id, &mut tasks)?;

    let mut index = 0usize;
    while index < tasks.len() {
        changed |= reconcile_deferred_lightweight_publish(&mut run, &mut tasks, index)?;
        index += 1;
    }

    let completion_gate_issues = if tasks
        .iter()
        .all(|task| task.status == CoordinationTaskStatus::Completed)
    {
        run_completion_gate_issues(&run, &tasks)?
    } else {
        Vec::new()
    };
    let mut new_status = derive_run_status(&tasks, &completion_gate_issues);
    if matches!(
        new_status,
        CoordinationRunStatus::Active | CoordinationRunStatus::WaitingForArtifacts
    ) && has_open_interrupts(run_id)
    {
        new_status = CoordinationRunStatus::WaitingForPolicy;
    }
    new_status = merge_derived_run_status(&run.status, new_status);
    if run.status != new_status {
        if new_status == CoordinationRunStatus::Failed && !completion_gate_issues.is_empty() {
            append_event(
                run_id,
                "run_completion_blocked",
                None,
                &format!(
                    "Run failed completion gate: {}",
                    completion_gate_issues.join(" | ")
                ),
            )?;
        }
        run.status = new_status;
        changed = true;
    }

    if changed {
        run.updated_at = Utc::now();
        write_json_atomic(&run_path(&run.run_id), &run)?;
        index_run(&run);
        sync_run_summary_after_task_update(run_id, run.updated_at)?;
    }
    Ok(changed)
}
