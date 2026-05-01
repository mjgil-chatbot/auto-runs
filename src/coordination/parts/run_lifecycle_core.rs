pub fn ensure_workspace_provider_capability_registry(run: &CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::DesignConsensus {
        return Ok(());
    }

    let target_path = Path::new(&run.workspace_path).join("workflow-registry/providers.json");
    const EMBEDDED_PROVIDER_CAPABILITY_REGISTRY_JSON: &str =
        include_str!("../../../workflow-registry/providers.json");
    let registry_contents = fs::read_to_string(PROVIDER_CAPABILITY_REGISTRY_PATH)
        .unwrap_or_else(|_| EMBEDDED_PROVIDER_CAPABILITY_REGISTRY_JSON.to_string());
    if fs::read_to_string(&target_path)
        .map(|existing| existing == registry_contents)
        .unwrap_or(false)
    {
        return Ok(());
    }

    let parent = target_path
        .parent()
        .context("provider capability registry path missing parent")?;
    fs::create_dir_all(parent)?;
    fs::write(&target_path, registry_contents)
        .with_context(|| format!("failed to write {}", target_path.display()))?;
    Ok(())
}
pub(crate) fn persist_selected_tasks(
    run_id: &str,
    tasks: &[CoordinationTask],
    task_ids: &[String],
) -> Result<()> {
    for task_id in task_ids {
        let Some(task) = tasks.iter().find(|task| task.task_id == *task_id) else {
            continue;
        };
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
    }
    Ok(())
}
pub fn load_run_snapshot(run_id: &str) -> Result<CoordinationRunSnapshot> {
    reconcile_run_state(run_id)?;
    refresh_claim_state(run_id)?;
    let mut run = load_run(run_id)?;
    let mut tasks = load_tasks(run_id)?;
    let cooldowns_changed = prune_expired_provider_cooldowns(&mut run, Utc::now());
    if run.status == CoordinationRunStatus::Canceled {
        if let Err(err) = cleanup_perf_workspace(&run) {
            warn!(
                run_id = %run.run_id,
                error = %err,
                "failed to clean disposable perf worktree for canceled run"
            );
        }
        return Ok(CoordinationRunSnapshot { run, tasks });
    }
    // Council runs keep their own ledger state, but open interrupts still gate execution.
    if run.workflow_mode == AutonomousWorkflowMode::FreeformCouncil {
        let new_status = if matches!(
            run.status,
            CoordinationRunStatus::Completed
                | CoordinationRunStatus::Failed
                | CoordinationRunStatus::Canceled
        ) {
            run.status.clone()
        } else if has_open_interrupts(run_id) {
            CoordinationRunStatus::WaitingForPolicy
        } else if run.status == CoordinationRunStatus::WaitingForPolicy {
            CoordinationRunStatus::Active
        } else {
            run.status.clone()
        };
        if cooldowns_changed || run.status != new_status {
            run.status = new_status;
            run.updated_at = Utc::now();
            write_json_atomic(&run_path(run_id), &run)?;
            index_run(&run);
        }
        return Ok(CoordinationRunSnapshot { run, tasks });
    }
    let unblocked_task_ids = unblock_runnable_task_ids(&mut tasks);
    let tasks_changed = !unblocked_task_ids.is_empty();
    let retried_failed_tasks = requeue_failed_tasks_for_retry(run_id, &run, &mut tasks)?;
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
    let run_changed = run.status != new_status;
    run.status = new_status;
    if tasks_changed || retried_failed_tasks || run_changed || cooldowns_changed {
        run.updated_at = Utc::now();
        write_json_atomic(&run_path(run_id), &run)?;
        index_run(&run);
        persist_selected_tasks(run_id, &tasks, &unblocked_task_ids)?;
        if run_changed
            && run.status == CoordinationRunStatus::Failed
            && !completion_gate_issues.is_empty()
        {
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
    }
    if run.status == CoordinationRunStatus::Failed {
        let _restart_lock = acquire_failed_run_restart_lock(run_id)?;
        run = load_run(run_id)?;
        if run.status == CoordinationRunStatus::Failed {
            match maybe_auto_fix_failed_run(run_id, &mut run) {
                Ok(true) => {
                    return restart_failed_tasks_with_mode_locked(
                        run_id,
                        RestartMode::AutoSelfHeal,
                    )
                }
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        run_id = %run.run_id,
                        error = %err,
                        "automatic self-heal failed while scheduling restart"
                    );
                    let _ = append_event(
                        run_id,
                        "run_auto_fix_restart_failed",
                        None,
                        &format!("Automatic self-heal failed before restart: {err}"),
                    );
                }
            }
        }
    }
    if matches!(
        run.status,
        CoordinationRunStatus::Completed
            | CoordinationRunStatus::Failed
            | CoordinationRunStatus::Canceled
    ) {
        if let Err(err) = cleanup_perf_workspace(&run) {
            warn!(
                run_id = %run.run_id,
                error = %err,
                "failed to clean disposable perf worktree for terminal run"
            );
        }
    }
    Ok(CoordinationRunSnapshot { run, tasks })
}
pub fn update_run_max_agents(run_id: &str, max_agents: u32) -> Result<()> {
    let mut run = load_run(run_id)?;
    let clamped = if matches!(
        run.workflow_mode,
        AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
    ) {
        1
    } else {
        max_agents.clamp(1, 8)
    };
    if run.max_agents != clamped {
        run.max_agents = clamped;
        run.updated_at = Utc::now();
        write_json_atomic(&run_path(run_id), &run)?;
    }
    Ok(())
}
fn normalized_max_agents(workflow_mode: &AutonomousWorkflowMode, max_agents: u32) -> u32 {
    if matches!(
        workflow_mode,
        AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
    ) {
        1
    } else {
        max_agents.max(1)
    }
}
pub(crate) fn restart_failed_tasks_with_mode(
    run_id: &str,
    restart_mode: RestartMode,
) -> Result<CoordinationRunSnapshot> {
    let _restart_lock = acquire_failed_run_restart_lock(run_id)?;
    restart_failed_tasks_with_mode_locked(run_id, restart_mode)
}
pub(crate) fn restart_failed_tasks_with_mode_locked(
    run_id: &str,
    restart_mode: RestartMode,
) -> Result<CoordinationRunSnapshot> {
    let mut mutation = begin_mutation_batch(
        run_id,
        None,
        CoordinationMutationKind::RestartFailedTasks,
        "Restarting failed tasks and rebasing restart generations.",
    )?;
    refresh_claim_state(run_id)?;
    let mut run = load_run(run_id)?;
    let mut tasks = load_tasks(run_id)?;
    let now = Utc::now();
    let new_generation = tasks
        .iter()
        .map(|task| task.execution_generation)
        .max()
        .unwrap_or(default_execution_generation())
        .saturating_add(1);
    let mut restarted = 0usize;
    let mut restarted_task_ids = BTreeSet::new();
    if refresh_run_provider_cooldowns(&mut run, &tasks, now)? {
        run.updated_at = now;
        write_json_atomic(&run_path(run_id), &run)?;
    }
    let repair_task_id = maybe_prepare_post_failure_repair_task(
        run_id,
        &mut run,
        &mut tasks,
        new_generation,
        now,
        restart_mode,
    )?;
    if tasks.iter().any(|task| {
        matches!(
            task.status,
            CoordinationTaskStatus::Failed | CoordinationTaskStatus::Canceled
        )
    }) || (run.status == CoordinationRunStatus::Failed
        && tasks
            .iter()
            .all(|task| task.status == CoordinationTaskStatus::Completed))
    {
        let mut workspace_repaired =
            repair_perf_workspace_for_manual_restart(run_id, &mut run, &mut tasks, now)?;
        workspace_repaired |=
            repair_lightweight_workspace_for_manual_restart(run_id, &mut run, &mut tasks, now)?;
        workspace_repaired |= repair_github_branch_fix_workspace_for_manual_restart(
            run_id, &mut run, &mut tasks, now,
        )?;
        if workspace_repaired {
            run.updated_at = now;
            write_json_atomic(&run_path(run_id), &run)?;
            index_run(&run);
        }
    }

    // First pass: restart tasks with Failed or Canceled status.
    for task in tasks.iter_mut() {
        let Some(archived_generation) = prepare_task_for_manual_restart(task, now, new_generation)
        else {
            continue;
        };
        if let Some(repair_task_id) = repair_task_id.as_ref() {
            task.dependencies = vec![repair_task_id.clone()];
            task.status = CoordinationTaskStatus::Blocked;
        }

        persist_restarted_task(run_id, task, archived_generation)?;
        append_event(
            run_id,
            "task_restarted",
            Some(&task.task_id),
            &format!(
                "Restarted failed task: {}{}",
                task.title,
                format_attempt_suffix(task)
            ),
        )?;
        restarted_task_ids.insert(task.task_id.clone());
        restarted += 1;
    }

    // Second pass: if the run failed at the completion gate (all tasks Completed
    // but some have bad artifact outcomes), reset those specific tasks so they
    // get re-executed.
    if restarted == 0
        && run.status == CoordinationRunStatus::Failed
        && tasks
            .iter()
            .all(|t| t.status == CoordinationTaskStatus::Completed)
    {
        for task in tasks.iter_mut() {
            if completed_task_needs_manual_restart(run_id, task)? {
                let archived_generation = task.execution_generation;
                task.execution_generation = new_generation;
                task.dependency_generation = Some(new_generation);
                task.status = CoordinationTaskStatus::Queued;
                task.claimed_by = None;
                task.artifact_summary = None;
                task.failure_kind = None;
                task.recovery_action = None;
                task.updated_at = now;
                if task.attempt_count >= task.max_attempts.max(1) {
                    task.max_attempts = manual_restart_max_attempts(task);
                }
                if let Some(repair_task_id) = repair_task_id.as_ref() {
                    task.dependencies = vec![repair_task_id.clone()];
                    task.status = CoordinationTaskStatus::Blocked;
                }
                persist_restarted_task(run_id, task, archived_generation)?;
                append_event(
                    run_id,
                    "task_restarted",
                    Some(&task.task_id),
                    &format!(
                        "Restarted gate-failed task: {}{}",
                        task.title,
                        format_attempt_suffix(task)
                    ),
                )?;
                restarted_task_ids.insert(task.task_id.clone());
                restarted += 1;
            }
        }
    }

    if !restarted_task_ids.is_empty() {
        restarted += reset_downstream_tasks_for_restart(
            run_id,
            &mut tasks,
            &restarted_task_ids,
            new_generation,
            now,
        )?;
    }

    if restarted > 0 {
        run.status = CoordinationRunStatus::Active;
        run.updated_at = now;
        write_json_atomic(&run_path(run_id), &run)?;
        index_run(&run);
        append_event(
            run_id,
            "run_restarted_failed_tasks",
            None,
            &format!("Restarted {} failed task(s).", restarted),
        )?;
    }

    complete_mutation_batch(&mut mutation)?;
    load_run_snapshot(run_id)
}
/// Find the most recent coordination run whose `root_chat_id` matches the given session ID.
/// Returns the run_id if found.
pub fn find_latest_run_for_session(session_id: &str) -> Option<String> {
    let root = coordination_root_dir();
    let mut best: Option<(chrono::DateTime<Utc>, String)> = None;
    let entries = fs::read_dir(&root).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let run_file = entry.path().join("run.json");
        let Ok(data) = fs::read_to_string(&run_file) else {
            continue;
        };
        if let Ok(run) = serde_json::from_str::<CoordinationRun>(&data) {
            if run.root_chat_id == session_id {
                if best
                    .as_ref()
                    .map(|(ts, _)| run.updated_at > *ts)
                    .unwrap_or(true)
                {
                    best = Some((run.updated_at, run.run_id.clone()));
                }
            }
        }
    }
    best.map(|(_, id)| id)
}
