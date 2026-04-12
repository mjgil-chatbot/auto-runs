pub fn make_task(
    run: &CoordinationRun,
    task_id: &str,
    parent_task_id: Option<String>,
    kind: CoordinationTaskKind,
    assigned_provider: Option<String>,
    title: String,
    prompt: String,
    allowed_tools: Vec<String>,
    read_scope: Vec<String>,
    write_scope: Vec<String>,
    dependencies: Vec<String>,
    status: CoordinationTaskStatus,
) -> CoordinationTask {
    let now = Utc::now();
    let acceptance_checks = seeded_acceptance_checks(&kind, &write_scope);
    CoordinationTask {
        provider_hint: None,
        required_role: None,
        task_category: None,
        task_id: task_id.to_string(),
        run_id: run.run_id.clone(),
        parent_task_id,
        kind,
        assigned_provider,
        iteration: None,
        title,
        prompt,
        cwd: perf_workspace_path_for_run(run),
        allowed_tools,
        read_scope,
        write_scope,
        acceptance_checks,
        review_metadata: None,
        dependencies,
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: default_task_max_attempts(),
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(&kind)),
        status,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    }
}
pub fn provider_prompt_text_for_run(run: &CoordinationRun, text: &str) -> String {
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc
        && !run.workspace_path.trim().is_empty()
    {
        text.replace(&run.workspace_path, ".")
    } else {
        text.to_string()
    }
}
pub fn workspace_path_for_run(run: &CoordinationRun, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(&run.workspace_path).join(path)
    }
}
