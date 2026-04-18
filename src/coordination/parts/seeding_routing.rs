pub fn seed_code_review_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let survey_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let materialize_id = Uuid::new_v4().to_string();
    let output_path = run.design_doc_path.clone();
    let scope_description =
        code_review::describe_scope(run.review_scope.as_ref().unwrap_or(&ReviewScope::Worktree));
    let mut materialize = make_task(
        run,
        &materialize_id,
        Some(plan_id.clone()),
        CoordinationTaskKind::MaterializeDesignDoc,
        None,
        "Write code review document".to_string(),
        format!(
            "Write the final code review document to `{}` using the completed review chunk artifacts.",
            output_path
        ),
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["artifacts/**".to_string()],
        vec![output_path.clone()],
        vec![plan_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    materialize.acceptance_checks = vec![
        format!("The final review document is written to {}.", output_path),
        "Findings are severity-ordered and cite file:line references.".to_string(),
    ];

    vec![
        make_task(
            run,
            &survey_id,
            None,
            CoordinationTaskKind::SurveyWorkspace,
            None,
            "Survey workspace".to_string(),
            format!(
                "Survey the workspace for code review context relevant to this review scope: {}",
                scope_description
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string()],
            Vec::new(),
            Vec::new(),
            CoordinationTaskStatus::Queued,
        ),
        make_task(
            run,
            &plan_id,
            Some(survey_id.clone()),
            CoordinationTaskKind::PlanCodeReview,
            None,
            "Plan code review chunks".to_string(),
            format!(
                "Plan a code review for {} and emit read-only review chunks using `{}`.",
                scope_description,
                code_review::REVIEW_PROFILE_PATH
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string(), "artifacts/**".to_string()],
            Vec::new(),
            vec![survey_id],
            CoordinationTaskStatus::Blocked,
        ),
        materialize,
    ]
}
pub fn persist_seed_code_review_metadata(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
) -> Result<()> {
    let scope_description =
        code_review::describe_scope(run.review_scope.as_ref().unwrap_or(&ReviewScope::Worktree));
    let fix_mode = run.review_fix_mode.clone().unwrap_or_default();
    for task in tasks {
        if task.kind != CoordinationTaskKind::MaterializeDesignDoc {
            continue;
        }
        let output_path = task
            .write_scope
            .first()
            .cloned()
            .unwrap_or_else(|| run.design_doc_path.clone());
        write_review_task_metadata(
            &run.run_id,
            &task.task_id,
            &code_review::ReviewTaskMetadata {
                scope_description: scope_description.clone(),
                dimensions: run.review_dimensions.clone().unwrap_or_default(),
                chunk: Vec::new(),
                automated_checks: Vec::new(),
                profile_path: code_review::REVIEW_PROFILE_PATH.to_string(),
                profile_warnings: Vec::new(),
                fix_mode: Some(fix_mode.clone()),
                output_path: Some(output_path.clone()),
            },
        )?;
    }
    Ok(())
}
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
pub fn push_unique_provider(providers: &mut Vec<String>, provider_id: &str) {
    if !providers.iter().any(|existing| existing == provider_id) {
        providers.push(provider_id.to_string());
    }
}
pub fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "claude" => "Claude Opus",
        "glm" => "GLM 5.1",
        "codex" => "GPT-5.4",
        "minimax" => "MiniMax",
        "gemini" => "gemini-3-flash-preview",
        "qwen" => "Qwen 3.6 Free",
        _ => "Provider",
    }
}
pub fn prompt_workspace_root_for_run(run: &CoordinationRun) -> &str {
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        "."
    } else {
        run.workspace_path.as_str()
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
pub fn sanitize_filename_component(raw: &str) -> String {
    let mut result = String::new();
    let mut last_was_separator = false;

    for ch in raw.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            last_was_separator = false;
            ch.to_ascii_lowercase()
        } else if matches!(ch, '.' | '-' | '_') {
            last_was_separator = false;
            ch
        } else {
            if !last_was_separator && !result.is_empty() {
                result.push('-');
            }
            last_was_separator = true;
            continue;
        };
        result.push(normalized);
    }

    let trimmed = result.trim_matches(|ch| ch == '-' || ch == '_' || ch == '.');
    if trimmed.is_empty() {
        "design".to_string()
    } else {
        trimmed.to_string()
    }
}
