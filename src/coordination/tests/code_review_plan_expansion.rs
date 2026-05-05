#[test]
fn expand_code_review_plan_generates_read_only_review_tasks() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let workspace =
        std::env::temp_dir().join(format!("rust-chatbot-review-plan-{}", Uuid::new_v4()));
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::create_dir_all(workspace.join("docs")).unwrap();
    fs::write(
        workspace.join("src/lib.rs"),
        "pub fn reviewed() -> anyhow::Result<()> {\n    Ok(())\n}\n",
    )
    .unwrap();
    fs::write(
        workspace.join("docs/review-profile.md"),
        "# Review Profile\n",
    )
    .unwrap();

    let run_id = format!("review-plan-{}", Uuid::new_v4());
    let mut run = CoordinationRun {
        run_id: run_id.clone(),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "claude".to_string(),
        goal: "review the workspace".to_string(),
        workflow_mode: AutonomousWorkflowMode::CodeReview,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: Some(ReviewScope::Worktree),
        review_dimensions: Some(vec!["architecture".to_string()]),
        review_fix_mode: Some(ReviewFixMode::ReportOnly),
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: BTreeSet::new(),
    };
    let tasks = seed_initial_tasks(&run);
    run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    fs::create_dir_all(artifacts_dir(&run_id)).unwrap();
    write_json_atomic(&run_path(&run_id), &run).unwrap();
    for task in &tasks {
        write_json_atomic(&task_path(&run_id, &task.task_id), task).unwrap();
    }
    persist_seed_code_review_metadata(&run, &tasks).unwrap();

    let plan_task = tasks[1].clone();
    let plan_artifact = execute_plan_code_review(&plan_task).expect("plan should succeed");
    expand_code_review_plan(&run_id, &plan_task, &plan_artifact.body).unwrap();

    let generated = load_tasks(&run_id)
        .unwrap()
        .into_iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .filter(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .collect::<Vec<_>>();
    assert!(!generated.is_empty());
    assert!(generated.iter().all(|task| task.write_scope.is_empty()));
    let plan: code_review::CodeReviewPlanArtifact =
        serde_json::from_str(&plan_artifact.body).unwrap();
    let providers = all_provider_names();
    for (index, review_plan) in plan.review_tasks.iter().enumerate() {
        let task = generated
            .iter()
            .find(|task| task.title == review_plan.title)
            .unwrap();
        assert_eq!(
            task.assigned_provider.as_deref(),
            Some(providers[index % providers.len()].as_str())
        );
    }

    let materialize = load_tasks(&run_id)
        .unwrap()
        .into_iter()
        .find(|task| task.kind == CoordinationTaskKind::MaterializeDesignDoc)
        .expect("materialize task should exist");
    assert_eq!(materialize.dependencies.len(), generated.len());

    let _ = fs::remove_dir_all(run_dir(&run_id));
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn expand_code_review_plan_repairs_missing_review_children() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let workspace = std::env::temp_dir().join(format!(
        "rust-chatbot-review-plan-repair-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::create_dir_all(workspace.join("docs")).unwrap();
    fs::write(workspace.join("src/lib.rs"), "pub fn reviewed() {}\n").unwrap();
    fs::write(
        workspace.join("docs/review-profile.md"),
        "# Review Profile\n",
    )
    .unwrap();

    let run_id = format!("review-plan-repair-{}", Uuid::new_v4());
    let mut run = CoordinationRun {
        run_id: run_id.clone(),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "claude".to_string(),
        goal: "review the workspace".to_string(),
        workflow_mode: AutonomousWorkflowMode::CodeReview,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: Some(ReviewScope::Worktree),
        review_dimensions: Some(vec!["architecture".to_string()]),
        review_fix_mode: Some(ReviewFixMode::ReportOnly),
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: BTreeSet::new(),
    };
    let tasks = seed_initial_tasks(&run);
    run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    fs::create_dir_all(artifacts_dir(&run_id)).unwrap();
    write_json_atomic(&run_path(&run_id), &run).unwrap();
    for task in &tasks {
        write_json_atomic(&task_path(&run_id, &task.task_id), task).unwrap();
    }
    persist_seed_code_review_metadata(&run, &tasks).unwrap();

    let plan_task = tasks[1].clone();
    let plan_artifact = execute_plan_code_review(&plan_task).unwrap();
    expand_code_review_plan(&run_id, &plan_task, &plan_artifact.body).unwrap();

    let generated = load_tasks(&run_id)
        .unwrap()
        .into_iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .filter(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .collect::<Vec<_>>();
    assert!(!generated.is_empty());
    fs::remove_file(task_path(&run_id, &generated[0].task_id)).unwrap();

    let materialize_before = load_tasks(&run_id)
        .unwrap()
        .into_iter()
        .find(|task| task.kind == CoordinationTaskKind::MaterializeDesignDoc)
        .unwrap();
    let mut materialize_rewired_back = materialize_before.clone();
    materialize_rewired_back.dependencies = vec![plan_task.task_id.clone()];
    write_json_atomic(
        &task_path(&run_id, &materialize_rewired_back.task_id),
        &materialize_rewired_back,
    )
    .unwrap();

    expand_code_review_plan(&run_id, &plan_task, &plan_artifact.body).unwrap();

    let repaired = load_tasks(&run_id).unwrap();
    let generated_after = repaired
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .filter(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .collect::<Vec<_>>();
    assert_eq!(generated_after.len(), generated.len());
    let materialize_after = repaired
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::MaterializeDesignDoc)
        .unwrap();
    assert_eq!(materialize_after.dependencies.len(), generated_after.len());

    let _ = fs::remove_dir_all(run_dir(&run_id));
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn expand_review_task_repairs_missing_follow_up_review() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let run_id = format!("review-fix-repair-{}", Uuid::new_v4());
    let now = Utc::now();
    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    fs::create_dir_all(artifacts_dir(&run_id)).unwrap();

    let run = CoordinationRun {
        run_id: run_id.clone(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "fix the issue".to_string(),
        workflow_mode: AutonomousWorkflowMode::SingleProvider,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: now,
        updated_at: now,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: BTreeSet::new(),
    };
    write_json_atomic(&run_path(&run_id), &run).unwrap();

    let review_task = CoordinationTask {
        task_id: "review".to_string(),
        run_id: run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Review".to_string(),
        prompt: "Inspect the change".to_string(),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string()],
        read_scope: vec!["src/lib.rs".to_string()],
        write_scope: Vec::new(),
        acceptance_checks: vec!["Review current workspace".to_string()],
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::ReviewArtifacts,
        )),
        status: CoordinationTaskStatus::Completed,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };
    let apply_task = CoordinationTask {
        task_id: "apply".to_string(),
        run_id: run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ApplyApprovedEdits,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Apply".to_string(),
        prompt: "Apply approved edits".to_string(),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["src/lib.rs".to_string()],
        write_scope: vec!["src/lib.rs".to_string()],
        acceptance_checks: vec!["Patch applied".to_string()],
        review_metadata: None,
        dependencies: vec![review_task.task_id.clone()],
        execution_generation: review_task.execution_generation,
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::ApplyApprovedEdits,
        )),
        status: CoordinationTaskStatus::Blocked,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };
    write_json_atomic(&task_path(&run_id, &review_task.task_id), &review_task).unwrap();
    write_json_atomic(&task_path(&run_id, &apply_task.task_id), &apply_task).unwrap();

    let artifact = TaskArtifact {
        run_id: run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "Review found a fix".to_string(),
        body: String::new(),
        completed_at: now,
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "Review found a fix".to_string(),
            details: "Need one patch".to_string(),
            changed_files: Vec::new(),
            tests_run: Vec::new(),
            warnings: Vec::new(),
            blockers: vec!["Fix required".to_string()],
            remaining_gaps: vec!["Need follow-up review".to_string()],
            design_requirements_satisfied: Vec::new(),
            validation_errors: Vec::new(),
            failure_kind: None,
            recovery_action: None,
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Patch src/lib.rs".to_string(),
                provider: Some("codex".to_string()),
                prompt: "Patch the file".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["src/lib.rs".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "src/lib.rs is updated".to_string(),
                    "no unrelated files change".to_string(),
                ],
            }],
        },
    };

    expand_review_task(&run_id, &review_task, &artifact).unwrap();

    let first_children = load_tasks(&run_id).unwrap();
    let follow_up = first_children
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(review_task.task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
                && task.title == "Follow-up review after Review"
        })
        .unwrap()
        .clone();
    fs::remove_file(task_path(&run_id, &follow_up.task_id)).unwrap();
    let mut apply_reset = load_task(&run_id, &apply_task.task_id).unwrap();
    apply_reset.dependencies = vec![review_task.task_id.clone()];
    write_json_atomic(&task_path(&run_id, &apply_reset.task_id), &apply_reset).unwrap();

    expand_review_task(&run_id, &review_task, &artifact).unwrap();

    let repaired = load_tasks(&run_id).unwrap();
    let repaired_follow_up = repaired
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(review_task.task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
                && task.title == "Follow-up review after Review"
        })
        .unwrap();
    let repaired_apply = repaired
        .iter()
        .find(|task| task.task_id == apply_task.task_id)
        .unwrap();
    assert_eq!(
        repaired_apply.dependencies,
        vec![repaired_follow_up.task_id.clone()]
    );

    let _ = fs::remove_dir_all(run_dir(&run_id));
}
