pub fn upsert_run(input: UpsertRunInput) -> Result<CoordinationRunSnapshot> {
    // Validate input at API boundary to catch configuration errors early
    validate_run_input(&input)?;

    fs::create_dir_all(coordination_root_dir())?;

    if let Some(run_id) = input.existing_run_id.as_ref() {
        if let Ok(mut existing) = load_run(run_id) {
            // Reject configuration edits once tasks have been seeded — the
            // existing task DAG carries cwd, provider, and workflow metadata
            // from the original config and would become stale.
            if !existing.task_ids.is_empty() {
                anyhow::bail!(
                    "cannot edit run {run_id}: tasks have already been seeded. \
                     Cancel this run and create a new one instead."
                );
            }
            let design_doc_path = if input.design_doc_path.trim().is_empty()
                || crate::is_generated_design_doc_path(&input.design_doc_path)
            {
                crate::default_design_doc_path_for_run(&input.workflow_mode, run_id)
            } else {
                input.design_doc_path.trim().to_string()
            };
            existing.goal = input.goal;
            existing.workspace_path = input.workspace_path;
            existing.execution_environment = input.execution_environment;
            existing.runsc_workspace_includes = input.runsc_workspace_includes;
            existing.workflow_mode = input.workflow_mode;
            existing.design_doc_path = design_doc_path;
            existing.completion_criteria = input.completion_criteria;
            existing.max_iterations = input.max_iterations.max(1);
            existing.updated_at = Utc::now();
            existing.max_agents = normalized_max_agents(&existing.workflow_mode, input.max_agents);
            existing.allow_write_tasks = input.allow_write_tasks;
            existing.require_review_before_apply = input.require_review_before_apply;
            existing.auto_retry_failed_tasks = input.auto_retry_failed_tasks;
            existing.notes = input.notes;
            existing.review_scope = input.review_scope;
            existing.review_dimensions = input.review_dimensions;
            existing.review_fix_mode = input.review_fix_mode;
            existing.council_turn_policy = input.council_turn_policy;
            existing.council_max_turns = input.council_max_turns;
            prepare_perf_workspace(&mut existing)?;
            prepare_github_branch_fix_workspace(&mut existing)?;
            reset_autonomous_runsc_workspace(run_id)?;
            ensure_workspace_provider_capability_registry(&existing)?;
            let _ = ensure_autonomous_runsc_workspace(&existing)?;
            write_json_atomic(&run_path(run_id), &existing)?;
            append_event(
                run_id,
                "run_updated",
                None,
                "Updated autonomous coordination run configuration.",
            )?;
            return load_run_snapshot(run_id);
        }
    }

    let run_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let design_doc_path = if input.design_doc_path.trim().is_empty()
        || crate::is_generated_design_doc_path(&input.design_doc_path)
    {
        crate::default_design_doc_path_for_run(&input.workflow_mode, &run_id)
    } else {
        input.design_doc_path.trim().to_string()
    };
    let max_agents = normalized_max_agents(&input.workflow_mode, input.max_agents);
    // Normalize provider to lowercase for consistent matching
    let normalized_provider = input.provider.trim().to_lowercase();
    let mut run = CoordinationRun {
        run_id: run_id.clone(),
        root_chat_id: input.root_chat_id,
        workspace_path: input.workspace_path.clone(),
        execution_environment: input.execution_environment,
        runsc_workspace_includes: input.runsc_workspace_includes,
        provider: normalized_provider,
        goal: input.goal.clone(),
        workflow_mode: input.workflow_mode,
        design_doc_path,
        completion_criteria: input.completion_criteria,
        max_iterations: input.max_iterations.max(1),
        status: CoordinationRunStatus::Active,
        created_at: now,
        updated_at: now,
        created_by_instance: input.created_by_instance,
        max_agents,
        allow_write_tasks: input.allow_write_tasks,
        require_review_before_apply: input.require_review_before_apply,
        auto_retry_failed_tasks: input.auto_retry_failed_tasks,
        notes: input.notes,
        review_scope: input.review_scope,
        review_dimensions: input.review_dimensions,
        review_fix_mode: input.review_fix_mode,
        council_turn_policy: input.council_turn_policy,
        council_max_turns: input.council_max_turns,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
       processed_mutation_batch_ids: BTreeSet::new(),
    };
    prepare_perf_workspace(&mut run)?;
    prepare_github_branch_fix_workspace(&mut run)?;

    fs::create_dir_all(tasks_dir(&run_id))?;
    fs::create_dir_all(claims_dir(&run_id))?;
    fs::create_dir_all(artifacts_dir(&run_id))?;

    // Council mode: create the council-specific on-disk layout instead of task DAG.
    if run.workflow_mode == AutonomousWorkflowMode::FreeformCouncil {
        seed_council_run(&run)?;
    }

    ensure_workspace_provider_capability_registry(&run)?;
    let _ = ensure_autonomous_runsc_workspace(&run)?;
    let tasks = seed_initial_tasks(&run);
    run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();

    write_json_atomic(&run_path(&run_id), &run)?;
    index_run(&run);
    for task in &tasks {
        write_json_atomic(&task_path(&run_id, &task.task_id), task)?;
        index_task(task);
    }
    refresh_lightweight_status_projection(&run_id)?;
    if run.workflow_mode == AutonomousWorkflowMode::CodeReview {
        persist_seed_code_review_metadata(&run, &tasks)?;
    }
    append_event(
        &run_id,
        "run_created",
        None,
        &if run.workflow_mode == AutonomousWorkflowMode::FreeformCouncil {
            "Created autonomous coordination run (Freeform Council).".to_string()
        } else {
            "Created autonomous coordination run.".to_string()
        },
    )?;

    load_run_snapshot(&run_id)
}

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

pub fn stop_run(run_id: &str, reason: &str) -> Result<CoordinationRunSnapshot> {
    refresh_claim_state(run_id)?;
    let mut run = load_run(run_id)?;
    let mut tasks = load_tasks(run_id)?;
    let now = Utc::now();
    let worker_pids_to_stop = collect_claim_worker_pids_for_stop(run_id);

    run.status = CoordinationRunStatus::Canceled;
    run.updated_at = now;
    write_json_atomic(&run_path(run_id), &run)?;
    index_run(&run);

    for task in tasks.iter_mut() {
        if matches!(
            task.status,
            CoordinationTaskStatus::Completed
                | CoordinationTaskStatus::Failed
                | CoordinationTaskStatus::Canceled
        ) {
            continue;
        }

        task.status = CoordinationTaskStatus::Canceled;
        task.claimed_by = None;
        task.artifact_summary = Some(reason.to_string());
        task.max_attempts = task.attempt_count.max(1);
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        append_event(
            run_id,
            "task_canceled",
            Some(&task.task_id),
            &format!("Canceled task during run stop: {}", task.title),
        )?;
    }

    if let Ok(entries) = fs::read_dir(claims_dir(run_id)) {
        for entry in entries.flatten() {
            if entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    stop_detached_workers(&worker_pids_to_stop);
    if let Err(err) = cleanup_perf_workspace(&run) {
        warn!(
            run_id = %run.run_id,
            error = %err,
            "failed to clean disposable perf worktree while stopping run"
        );
    }

    append_event(run_id, "run_canceled", None, reason)?;
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

pub fn restart_failed_tasks(run_id: &str) -> Result<CoordinationRunSnapshot> {
    restart_failed_tasks_with_mode(run_id, RestartMode::Manual)
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

pub fn mark_task_failed_for_restart(
    run_id: &str,
    task_id: &str,
    reason: &str,
) -> Result<CoordinationRunSnapshot> {
    refresh_claim_state(run_id)?;
    let mut run = load_run(run_id)?;
    let mut task = load_task(run_id, task_id)?;
    if task.status != CoordinationTaskStatus::Completed {
        anyhow::bail!(
            "task {} for run {} is not completed and cannot be marked failed for restart",
            task_id,
            run_id
        );
    }
    let now = Utc::now();
    task.status = CoordinationTaskStatus::Failed;
    task.claimed_by = None;
    task.updated_at = now;
    if task.artifact_summary.is_none() {
        task.artifact_summary = Some("Manually marked failed before restart.".to_string());
    }
    write_json_atomic(&task_path(run_id, task_id), &task)?;
    index_task(&task);
    run.status = CoordinationRunStatus::Failed;
    run.auto_retry_failed_tasks = false;
    run.updated_at = now;
    write_json_atomic(&run_path(run_id), &run)?;
    index_run(&run);
    append_event(
        run_id,
        "task_marked_failed",
        Some(task_id),
        &format!(
            "Marked task failed before restart: {}. {}",
            task.title,
            reason.trim()
        ),
    )?;
    load_run_snapshot(run_id)
}

pub fn set_task_assigned_provider(run_id: &str, task_id: &str, provider: &str) -> Result<()> {
    refresh_claim_state(run_id)?;
    let canonical_provider = normalize_provider_name(Some(provider))
        .with_context(|| format!("unknown provider '{provider}'"))?;
    let mut run = load_run(run_id)?;
    let mut task = load_task(run_id, task_id)?;
    if matches!(
        task.status,
        CoordinationTaskStatus::Running
            | CoordinationTaskStatus::Claiming
            | CoordinationTaskStatus::ArtifactReady
    ) {
        anyhow::bail!(
            "task {} for run {} is currently active and cannot be reassigned",
            task_id,
            run_id
        );
    }
    if task.assigned_provider.as_deref() == Some(canonical_provider.as_str()) {
        return Ok(());
    }

    let now = Utc::now();
    task.assigned_provider = Some(canonical_provider.clone());
    task.updated_at = now;
    write_json_atomic(&task_path(run_id, task_id), &task)?;
    index_task(&task);

    run.updated_at = now;
    write_json_atomic(&run_path(run_id), &run)?;
    index_run(&run);

    append_event(
        run_id,
        "task_provider_reassigned",
        Some(task_id),
        &format!(
            "Updated assigned provider for {} to {}.",
            task.title, canonical_provider
        ),
    )?;
    Ok(())
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

pub fn drive_run_for_instance(run_id: &str, instance_pid: u32, provider: &str) -> Result<bool> {
    reconcile_run_state(run_id)?;
    refresh_claim_state(run_id)?;
    let run = load_run(run_id)?;
    if matches!(
        run.status,
        CoordinationRunStatus::Completed
            | CoordinationRunStatus::Failed
            | CoordinationRunStatus::Canceled
    ) {
        return Ok(false);
    }
    if has_open_interrupts(run_id) {
        return Ok(false);
    }
    // Council runs use a separate ledger-based driver instead of the task DAG.
    if run.workflow_mode == AutonomousWorkflowMode::FreeformCouncil {
        return council::drive_council_turn(run_id, instance_pid);
    }

    if count_active_claims(run_id)? >= run.max_agents as usize {
        return Ok(false);
    }

    // Safety net: if no tasks are queued but some are blocked, try unblocking.
    // This recovers from cases where load_run_snapshot (the normal unblock path)
    // errored silently, leaving tasks stuck in blocked state.
    let mut tasks = load_tasks(run_id)?;
    let has_queued = tasks
        .iter()
        .any(|t| t.status == CoordinationTaskStatus::Queued);
    if !has_queued {
        let unblocked_task_ids = unblock_runnable_task_ids(&mut tasks);
        if !unblocked_task_ids.is_empty() {
            eprintln!(
                "drive_run_for_instance: recovered stalled run {} — unblocked tasks via safety net",
                run_id
            );
            persist_selected_tasks(run_id, &tasks, &unblocked_task_ids)?;
        }
    }

    let Some(task) = claim_next_runnable_task(run_id, instance_pid, provider)? else {
        return Ok(false);
    };

    if is_provider_backed(&task) {
        if let Err(err) = spawn_provider_task_process(&task, provider, instance_pid) {
            handle_launch_failure_after_claim(&task.run_id, &task.task_id, instance_pid, &err)?;
            return Err(err);
        }
        return Ok(true);
    }

    let artifact = match execute_builtin_task(&task) {
        Ok(artifact) => artifact,
        Err(err) => {
            if let Err(fail_err) = fail_task(run_id, &task.task_id, instance_pid, &err.to_string())
            {
                warn!(
                    run_id = %task.run_id,
                    task_id = %task.task_id,
                    error = %fail_err,
                    "failed to mark builtin task as failed after execution error"
                );
            }
            return Err(err);
        }
    };
    if let Err(err) = complete_task(run_id, &task.task_id, instance_pid, artifact) {
        if let Err(fail_err) = fail_task(run_id, &task.task_id, instance_pid, &err.to_string()) {
            warn!(
                run_id = %task.run_id,
                task_id = %task.task_id,
                error = %fail_err,
                "failed to mark builtin task as failed after completion error"
            );
        }
        return Err(err);
    }
    Ok(true)
}

// validate_run_input is defined in run_lifecycle_retry.rs
