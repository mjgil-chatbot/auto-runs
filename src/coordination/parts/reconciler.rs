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
