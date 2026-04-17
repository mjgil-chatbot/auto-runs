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
