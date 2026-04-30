use super::*;

const PERF_RECOVERY_CLEANUP_LABEL: &str = "git checkout -- . && git clean -fd";

fn perf_idea_display_number(iteration: u32) -> u32 {
    iteration + 1
}

pub fn recover_perf_idea_failure(
    run_id: &str,
    task: &mut CoordinationTask,
    instance_pid: u32,
    provider_name: &str,
    message: &str,
) -> Result<bool> {
    if !matches!(
        task.kind,
        CoordinationTaskKind::PerfApply
            | CoordinationTaskKind::PerfBench
            | CoordinationTaskKind::PerfEvaluate
    ) {
        return Ok(false);
    }

    let Some(iteration) = task.iteration else {
        return Ok(false);
    };

    let mut run = load_run(run_id)?;
    if ensure_perf_workspace_ready(&mut run)? {
        write_json_atomic(&run_path(run_id), &run)?;
        index_run(&run);
    }
    let cleanup_note = match cleanup_rejected_perf_workspace(&run) {
        Ok(()) => None,
        Err(err) => Some(format!(
            " Cleanup command `{}` also failed: {}",
            PERF_RECOVERY_CLEANUP_LABEL, err
        )),
    };
    let rejection_reason = format!(
        "{} failed and the coordinator rejected this idea. {}{}",
        perf_task_kind_label(task.kind),
        message.trim(),
        cleanup_note.as_deref().unwrap_or("")
    );

    if task.kind == CoordinationTaskKind::PerfApply {
        persist_completed_perf_task(
            run_id,
            task,
            Some(instance_pid),
            provider_name,
            TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedNoChange,
                summary: format!(
                    "Rejected idea {} after {} failure",
                    iteration + 1,
                    perf_task_kind_slug(task.kind)
                ),
                details: rejection_reason.clone(),
                warnings: vec![
                    "Coordinator converted perf task failure into a rejected idea.".to_string(),
                ],
                ..Default::default()
            },
            rejection_reason.clone(),
        )?;
    }

    let bench_json = serde_json::to_value(make_failed_perf_benchmark_artifact(
        Some(iteration),
        rejection_reason.clone(),
    ))?;
    let bench_body = serde_json::to_string(&bench_json)?;
    fs::write(format!("/tmp/perf_bench_{}.json", iteration), &bench_body)?;

    if task.kind == CoordinationTaskKind::PerfApply {
        if let Some(mut bench_task) =
            find_perf_iteration_task(run_id, CoordinationTaskKind::PerfBench, iteration)?
        {
            persist_completed_perf_task(
                run_id,
                &mut bench_task,
                None,
                provider_name,
                TaskExecutionReport {
                    outcome: CoordinationTaskOutcome::CompletedWithChanges,
                    summary: format!(
                        "Bench idea {}: skipped after apply failure",
                        perf_idea_display_number(iteration)
                    ),
                    details: bench_body.clone(),
                    changed_files: vec![format!("/tmp/perf_bench_{}.json", iteration)],
                    warnings: vec![
                        "Coordinator synthesized a bench result after the apply step failed."
                            .to_string(),
                    ],
                    ..Default::default()
                },
                bench_body.clone(),
            )?;
        }
    } else if task.kind == CoordinationTaskKind::PerfBench {
        persist_completed_perf_task(
            run_id,
            task,
            Some(instance_pid),
            provider_name,
            TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedWithChanges,
                summary: format!(
                    "Bench idea {}: release build failed",
                    perf_idea_display_number(iteration)
                ),
                details: bench_body.clone(),
                changed_files: vec![format!("/tmp/perf_bench_{}.json", iteration)],
                warnings: vec![
                    "Coordinator synthesized a rejected bench result after the bench step failed."
                        .to_string(),
                ],
                ..Default::default()
            },
            bench_body.clone(),
        )?;
    }

    let eval_json = serde_json::json!({
        "keep": false,
        "idea_index": iteration,
        "reason": rejection_reason,
        "improvement_pct": 0.0
    });
    let eval_body = serde_json::to_string(&eval_json)?;
    fs::write(format!("/tmp/perf_evaluate_{}.json", iteration), &eval_body)?;

    let mut eval_task = if task.kind == CoordinationTaskKind::PerfEvaluate {
        task.clone()
    } else {
        find_perf_iteration_task(run_id, CoordinationTaskKind::PerfEvaluate, iteration)?
            .context("missing PerfEvaluate task for failed perf idea")?
    };
    let eval_summary = format!(
        "Idea {} reverted: 0.0% — {}",
        perf_idea_display_number(iteration),
        rejection_reason
    );
    persist_completed_perf_task(
        run_id,
        &mut eval_task,
        (task.kind == CoordinationTaskKind::PerfEvaluate).then_some(instance_pid),
        provider_name,
        TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: eval_summary.clone(),
            details: eval_body.clone(),
            changed_files: vec![format!("/tmp/perf_evaluate_{}.json", iteration)],
            warnings: vec![
                "Coordinator synthesized a rejected perf evaluation after an idea-scoped task failed."
                    .to_string(),
            ],
            ..Default::default()
        },
        eval_body.clone(),
    )?;
    if task.kind == CoordinationTaskKind::PerfEvaluate {
        *task = eval_task.clone();
    }

    expand_perf_evaluate_task(run_id, &eval_task, &eval_body)?;
    sync_perf_run_summary_after_recovery(run_id)?;
    Ok(true)
}

fn cleanup_rejected_perf_workspace(run: &CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(());
    }
    let workspace = perf_workspace_path_for_run(run);
    let checkout = std::process::Command::new("git")
        .arg("checkout")
        .arg("--")
        .arg(".")
        .current_dir(&workspace)
        .output()
        .with_context(|| format!("failed to launch git checkout in {}", workspace))?;
    if !checkout.status.success() {
        anyhow::bail!(
            "git checkout exited with status {}: {}",
            checkout.status,
            String::from_utf8_lossy(&checkout.stderr).trim()
        );
    }

    let clean = std::process::Command::new("git")
        .arg("clean")
        .arg("-fd")
        .current_dir(&workspace)
        .output()
        .with_context(|| format!("failed to launch git clean in {}", workspace))?;
    if !clean.status.success() {
        anyhow::bail!(
            "git clean exited with status {}: {}",
            clean.status,
            String::from_utf8_lossy(&clean.stderr).trim()
        );
    }
    Ok(())
}

fn find_perf_iteration_task(
    run_id: &str,
    kind: CoordinationTaskKind,
    iteration: u32,
) -> Result<Option<CoordinationTask>> {
    Ok(load_tasks(run_id)?
        .into_iter()
        .find(|candidate| candidate.kind == kind && candidate.iteration == Some(iteration)))
}

fn perf_task_kind_label(kind: CoordinationTaskKind) -> &'static str {
    match kind {
        CoordinationTaskKind::PerfApply => "PerfApply",
        CoordinationTaskKind::PerfBench => "PerfBench",
        CoordinationTaskKind::PerfEvaluate => "PerfEvaluate",
        _ => "Perf task",
    }
}

fn perf_task_kind_slug(kind: CoordinationTaskKind) -> &'static str {
    match kind {
        CoordinationTaskKind::PerfApply => "perf_apply",
        CoordinationTaskKind::PerfBench => "perf_bench",
        CoordinationTaskKind::PerfEvaluate => "perf_evaluate",
        _ => "perf",
    }
}

fn persist_completed_perf_task(
    run_id: &str,
    task: &mut CoordinationTask,
    claimed_by: Option<u32>,
    provider_name: &str,
    report: TaskExecutionReport,
    body: String,
) -> Result<()> {
    let summary = if report.summary.trim().is_empty() {
        task.title.clone()
    } else {
        report.summary.trim().to_string()
    };
    let artifact = TaskArtifact {
        run_id: run_id.to_string(),
        task_id: task.task_id.clone(),
        provider: task
            .assigned_provider
            .clone()
            .or_else(|| Some(provider_name.to_string())),
        task_category: task.task_category,
        fallback_from: task.provider_hint.clone(),
        summary: summary.clone(),
        body,
        completed_at: Utc::now(),
        report: report.clone(),
    };

    task.status = CoordinationTaskStatus::Completed;
    task.claimed_by = claimed_by;
    task.artifact_summary = Some(format!("{} [{}]", summary, report.outcome.as_str()));
    task.updated_at = Utc::now();

    fs::create_dir_all(artifact_dir(run_id, &task.task_id))?;
    write_json_atomic(&artifact_result_path(run_id, &task.task_id), &artifact)?;
    fs::write(artifact_notes_path(run_id, &task.task_id), &artifact.body)?;
    let _ = fs::remove_file(artifact_materialized_files_path(run_id, &task.task_id));
    write_json_atomic(&task_path(run_id, &task.task_id), task)?;
    index_task(task);
    if claimed_by.is_some() {
        let _ = fs::remove_file(claim_path(run_id, &task.task_id));
    }
    append_event_with_provider(
        run_id,
        "task_completed",
        Some(&task.task_id),
        task.assigned_provider.as_deref().or(Some(provider_name)),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Completed: {} [{}]. {}",
            task.title,
            report.outcome.as_str(),
            summary
        ),
    )?;
    index_provider_usage(
        run_id,
        &task.task_id,
        task.assigned_provider.as_deref().unwrap_or(provider_name),
        task.task_category.as_ref(),
        &report.outcome,
        Some(&task.created_at),
        &task.updated_at,
        provider_cost_tier_for(task.assigned_provider.as_deref().unwrap_or(provider_name))
            .as_deref(),
    );
    Ok(())
}

fn sync_perf_run_summary_after_recovery(run_id: &str) -> Result<()> {
    let mut run = load_run(run_id)?;
    let mut tasks = load_tasks(run_id)?;
    let unblocked_task_ids = unblock_runnable_task_ids(&mut tasks);
    if !unblocked_task_ids.is_empty() {
        persist_selected_tasks(run_id, &tasks, &unblocked_task_ids)?;
    }
    let completion_gate_issues = if tasks
        .iter()
        .all(|task| task.status == CoordinationTaskStatus::Completed)
    {
        run_completion_gate_issues(&run, &tasks)?
    } else {
        Vec::new()
    };
    run.status = merge_derived_run_status(
        &run.status,
        derive_run_status(&tasks, &completion_gate_issues),
    );
    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;
    index_run(&run);
    Ok(())
}
