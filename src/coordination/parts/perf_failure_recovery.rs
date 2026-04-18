use super::*;
const PERF_RECOVERY_CLEANUP_LABEL: &str = "git checkout -- . && git clean -fd";
fn perf_idea_display_number(iteration: u32) -> u32 {
    iteration + 1
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
