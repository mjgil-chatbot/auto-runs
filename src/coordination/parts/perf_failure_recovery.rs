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
