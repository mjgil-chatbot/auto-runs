#[test]
fn lightweight_implementation_seeds_split_validate_review_pipeline() {
    let notes = lightweight_notes_with_validation_provider(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-bootstrap-tasks.md",
            "codex",
            &["codex".to_string(), "claude".to_string()],
        ),
        "gemini",
    );
    let run = CoordinationRun {
        run_id: "lightweight-run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/lightweight-bootstrap-design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    let tasks = seed_initial_tasks(&run);
    assert_eq!(tasks.len(), 4);
    assert_eq!(tasks[0].kind, CoordinationTaskKind::SurveyWorkspace);
    assert_eq!(tasks[1].kind, CoordinationTaskKind::PlanFlatTasks);
    assert_eq!(tasks[2].kind, CoordinationTaskKind::PlanFlatTasks);
    assert_eq!(tasks[3].kind, CoordinationTaskKind::ReviewArtifacts);
    assert_eq!(tasks[1].assigned_provider.as_deref(), Some("codex"));
    assert_eq!(tasks[2].assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(tasks[3].assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(tasks[2].task_category, Some(TaskCategory::Verify));
    assert!(tasks[1].allowed_tools.contains(&"edit".to_string()));
    assert!(tasks[1]
        .allowed_tools
        .contains(&"sequentialthinking".to_string()));
    assert!(tasks[2]
        .allowed_tools
        .contains(&"sequentialthinking".to_string()));
    assert!(tasks[1]
        .write_scope
        .contains(&"docs/lightweight-bootstrap-design.md".to_string()));
    assert!(tasks[1]
        .write_scope
        .contains(&"docs/autonomous/lightweight-bootstrap-tasks.md".to_string()));
    assert!(tasks[1]
        .prompt
        .contains("create the canonical design doc at `docs/lightweight-bootstrap-design.md`"));
    assert!(tasks[1]
        .prompt
        .contains("Generated implementation workers must not edit the current task list"));
    assert!(tasks[3]
        .write_scope
        .contains(&"docs/lightweight-bootstrap-design.md".to_string()));
    assert!(tasks[3]
        .write_scope
        .contains(&"docs/autonomous/lightweight-bootstrap-tasks.md".to_string()));
    assert!(tasks[3]
        .write_scope
        .contains(&"docs/autonomous/lightweight-bootstrap-tasks-pass-1.md".to_string()));
    assert!(tasks[3]
        .prompt
        .contains("create the next task list document"));
    assert!(tasks[3].prompt.contains("update the canonical design doc"));
}

#[test]
fn validate_lightweight_notes_rejects_unknown_validation_model() {
    let notes = [
        "lightweight_task_doc=docs/autonomous/lightweight-tasks.md",
        "lightweight_task_splitter=codex",
        "lightweight_validation_model=python",
        "lightweight_worker_models=codex",
    ]
    .join("\n");

    let error = validate_lightweight_notes(&notes, "docs/design.md")
        .expect_err("unknown validation model should fail validation");
    assert!(error
        .to_string()
        .contains("unknown lightweight validation model"));
}

#[test]
fn lightweight_config_from_run_preserves_mock_workflow_provider_settings() {
    let notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_validation_provider(
            &lightweight_notes_with_settings(
                "",
                "docs/autonomous/mock-lightweight-tasks.md",
                "claude",
                &["minimax".to_string(), "codex".to_string()],
            ),
            "gemini",
        ),
        LightweightRepoConfiguration::Rust,
    );
    let run = CoordinationRun {
        run_id: "lightweight-config".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    let config = lightweight_config_from_run(&run);
    assert_eq!(config.task_splitter_provider, "claude");
    assert_eq!(config.validation_provider, "gemini");
    assert_eq!(
        config.worker_providers,
        vec!["minimax".to_string(), "codex".to_string()]
    );
    assert_eq!(
        config.task_doc_path,
        "docs/autonomous/mock-lightweight-tasks.md"
    );
    assert_eq!(
        config.repo_configuration,
        LightweightRepoConfiguration::Rust
    );
}

#[test]
fn lightweight_generated_task_provider_cycles_across_mock_worker_models() {
    let mut run = CoordinationRun {
        run_id: "lightweight-workers".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "claude",
        &["minimax".to_string(), "codex".to_string()],
    );

    assert_eq!(
        lightweight_generated_task_provider(&run, None, 0).as_deref(),
        Some("minimax")
    );
    assert_eq!(
        lightweight_generated_task_provider(&run, None, 1).as_deref(),
        Some("codex")
    );
    assert_eq!(
        lightweight_generated_task_provider(&run, None, 2).as_deref(),
        Some("minimax")
    );
    assert_eq!(
        lightweight_generated_task_provider(&run, Some("gemini"), 0).as_deref(),
        Some("gemini")
    );
}

#[test]
fn lightweight_review_advances_current_task_doc_when_follow_up_work_exists() {
    let notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["codex".to_string()],
    );
    let mut run = CoordinationRun {
        run_id: "lightweight-review".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };
    let review_task = CoordinationTask {
        task_id: "review".to_string(),
        run_id: run.run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("claude".to_string()),
        task_category: Some(TaskCategory::Review),
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Validate".to_string(),
        prompt: lightweight_review_prompt(&run),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["**".to_string()],
        write_scope: vec![
            "docs/autonomous/lightweight-tasks.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-1.md".to_string(),
        ],
        acceptance_checks: Vec::new(),
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
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("claude".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "Needs another pass".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "Needs another pass".to_string(),
            details: "Created the next task list.".to_string(),
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Implement the next slice".to_string(),
                provider: None,
                prompt: "Do the next implementation slice.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "cargo check succeeds".to_string(),
                    "the slice is complete".to_string(),
                ],
            }],
            ..Default::default()
        },
    };
    let next_path =
        update_lightweight_task_doc_after_review(&mut run, &review_task, &artifact, &[]).unwrap();
    assert_eq!(next_path, "docs/autonomous/lightweight-tasks-pass-1.md");
    assert_eq!(
        lightweight_current_task_doc_path(&run),
        "docs/autonomous/lightweight-tasks-pass-1.md"
    );
    assert_eq!(
        lightweight_next_task_doc_path_for_run(&run),
        "docs/autonomous/lightweight-tasks-pass-2.md"
    );
}

#[test]
fn lightweight_seeded_mock_run_uses_configured_splitter_and_validator() {
    let fixture = CoordinationTestFixture::named("lightweight-mock-workflow-config");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_validation_provider(
            &lightweight_notes_with_settings(
                "",
                "docs/autonomous/mock-pass-tasks.md",
                "claude",
                &["minimax".to_string(), "codex".to_string()],
            ),
            "gemini",
        ),
        LightweightRepoConfiguration::Zig,
    );
    fixture.persist_run_with_tasks(&run, &[]);

    let seeded = seed_initial_tasks(&run);
    assert_eq!(seeded.len(), 4);
    assert_eq!(seeded[1].assigned_provider.as_deref(), Some("claude"));
    assert_eq!(seeded[2].assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(seeded[3].assigned_provider.as_deref(), Some("gemini"));
    assert!(seeded[1]
        .write_scope
        .contains(&"docs/autonomous/mock-pass-tasks.md".to_string()));
    assert!(seeded[1]
        .prompt
        .contains("Use the sequential thinking MCP tool"));
    assert!(seeded[2]
        .prompt
        .contains("Use the sequential thinking MCP tool"));
    assert!(seeded[1]
        .prompt
        .contains("Review owns task-list progression"));
}

#[test]
fn lightweight_status_projection_renders_sibling_status_doc() {
    let fixture = CoordinationTestFixture::named("lightweight-status-projection");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut completed = fixture.make_task(
        &run,
        "completed-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement parser slice",
        &["src/parser.zig"],
        &["src/parser.zig"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    completed.task_category = Some(TaskCategory::DeepWrite);
    completed.artifact_summary =
        Some("implemented parser slice [completed_with_changes]".to_string());
    let queued = fixture.make_task(
        &run,
        "queued-task",
        CoordinationTaskKind::GeneratedReadTask,
        Some("minimax"),
        "Verify parser slice",
        &["src/parser.zig"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[completed, queued]);

    refresh_lightweight_status_projection(&run.run_id).unwrap();

    let status_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2-status.md");
    let contents = std::fs::read_to_string(status_doc).unwrap();

    assert!(contents.contains("# Lightweight Status"));
    assert!(contents.contains("- Active Task List: `docs/autonomous/lightweight-tasks-pass-2.md`"));
    assert!(contents.contains("- Next Task List: `docs/autonomous/lightweight-tasks-pass-3.md`"));
    assert!(contents.contains("- completed: 1"));
    assert!(contents.contains("- queued: 1"));
    assert!(contents.contains("Implement parser slice"));
    assert!(contents.contains("Verify parser slice"));
}

#[test]
fn lightweight_broker_snapshot_persists_execution_task_state() {
    let fixture = CoordinationTestFixture::named("lightweight-broker-snapshot");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut completed = fixture.make_task(
        &run,
        "completed-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement parser slice",
        &["src/parser.zig"],
        &["src/parser.zig"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    completed.artifact_summary = Some("implemented parser slice".to_string());
    let blocked = fixture.make_task(
        &run,
        "blocked-task",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review parser slice",
        &["src/parser.zig"],
        &[],
        &["completed-task"],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[completed, blocked]);

    let snapshot = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let reloaded = load_lightweight_broker_snapshot(&run.run_id).unwrap();
    let execution_tasks = load_lightweight_execution_tasks(&run.run_id).unwrap();

    assert_eq!(snapshot.run_id, run.run_id);
    assert_eq!(
        reloaded.task_document.active_path,
        "docs/autonomous/lightweight-tasks-pass-2.md"
    );
    assert_eq!(
        reloaded.task_document.next_path,
        "docs/autonomous/lightweight-tasks-pass-3.md"
    );
    assert_eq!(
        reloaded.task_document.status_path,
        "docs/autonomous/lightweight-tasks-pass-2-status.md"
    );
    assert_eq!(reloaded.task_document.cycle, 2);
    assert_eq!(reloaded.execution_task_count, 2);
    assert_eq!(
        reloaded.execution_tasks_path,
        "lightweight_execution_tasks.json"
    );
    assert!(execution_tasks.iter().any(|task| {
        task.execution_task_id == "completed-task"
            && task.status == CoordinationTaskStatus::Completed
            && task.artifact_summary.as_deref() == Some("implemented parser slice")
    }));
    assert!(execution_tasks.iter().any(|task| {
        task.execution_task_id == "blocked-task"
            && task.status == CoordinationTaskStatus::Blocked
            && task.dependencies == vec!["completed-task".to_string()]
    }));
}

#[test]
fn lightweight_broker_snapshot_separates_source_and_execution_state_files() {
    let fixture = CoordinationTestFixture::named("lightweight-broker-separated-state");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    let task_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2.md");
    std::fs::create_dir_all(task_doc.parent().unwrap()).unwrap();
    std::fs::write(&task_doc, "# Core\n- [ ] Implement parser slice\n").unwrap();
    let completed = fixture.make_task(
        &run,
        "completed-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement parser slice",
        &["src/parser.zig"],
        &["src/parser.zig"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[completed]);

    let snapshot = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();

    assert_eq!(snapshot.source_tasks_path, "lightweight_source_tasks.json");
    assert_eq!(
        snapshot.execution_tasks_path,
        "lightweight_execution_tasks.json"
    );
    assert_eq!(snapshot.passes_path, "lightweight_passes.json");
    assert_eq!(snapshot.source_task_count, 1);
    assert_eq!(snapshot.execution_task_count, 1);
    assert_eq!(snapshot.pass_count, 1);
    assert!(lightweight_source_tasks_path(&run.run_id).is_file());
    assert!(lightweight_execution_tasks_path(&run.run_id).is_file());
    assert!(lightweight_passes_path(&run.run_id).is_file());
}

#[test]
fn lightweight_pass_records_persist_current_pass_membership() {
    let fixture = CoordinationTestFixture::named("lightweight-pass-records");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    let task_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2.md");
    std::fs::create_dir_all(task_doc.parent().unwrap()).unwrap();
    std::fs::write(
        &task_doc,
        "# Core\n- [ ] Implement parser slice\n- [ ] Verify parser slice\n",
    )
    .unwrap();
    let completed = fixture.make_task(
        &run,
        "completed-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement parser slice",
        &["src/parser.zig"],
        &["src/parser.zig"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let queued = fixture.make_task(
        &run,
        "queued-task",
        CoordinationTaskKind::GeneratedReadTask,
        Some("minimax"),
        "Verify parser slice",
        &["src/parser.zig"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[completed, queued]);

    let snapshot = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let passes = load_lightweight_pass_records(&run.run_id).unwrap();

    assert_eq!(snapshot.pass_count, 1);
    assert_eq!(passes.len(), 1);
    assert_eq!(passes[0].pass_id, "pass-2");
    assert_eq!(passes[0].cycle, 2);
    assert_eq!(
        passes[0].task_doc_path,
        "docs/autonomous/lightweight-tasks-pass-2.md"
    );
    assert_eq!(passes[0].status, LightweightPassStatus::Active);
    assert_eq!(passes[0].source_task_ids.len(), 2);
    assert_eq!(
        passes[0].execution_task_ids,
        vec!["completed-task".to_string(), "queued-task".to_string()]
    );
}

#[test]
fn lightweight_broker_snapshot_parses_source_tasks_from_checklist_doc() {
    let fixture = CoordinationTestFixture::named("lightweight-source-task-parser");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    fixture.persist_run_with_tasks(&run, &[]);

    let task_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2.md");
    std::fs::create_dir_all(task_doc.parent().unwrap()).unwrap();
    std::fs::write(
        &task_doc,
        "# Parser Work\n\n## Core\n- [ ] Implement parser slice\n- [x] Verify parser slice\n",
    )
    .unwrap();

    let snapshot = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let source_tasks = load_lightweight_source_tasks(&run.run_id).unwrap();

    assert_eq!(snapshot.source_task_count, 2);
    assert_eq!(snapshot.source_tasks_path, "lightweight_source_tasks.json");
    assert_eq!(
        source_tasks[0].source_task_id,
        "source-task-parser-work--core--implement-parser-slice"
    );
    assert_eq!(
        source_tasks[0].section_path,
        vec!["Parser Work".to_string(), "Core".to_string()]
    );
    assert!(!source_tasks[0].completed);
    assert!(source_tasks[1].completed);
}

#[test]
fn lightweight_source_task_ids_are_stable_across_reorder() {
    let fixture = CoordinationTestFixture::named("lightweight-source-task-stability");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    fixture.persist_run_with_tasks(&run, &[]);

    let task_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2.md");
    std::fs::create_dir_all(task_doc.parent().unwrap()).unwrap();
    std::fs::write(
        &task_doc,
        "# Parser Work\n\n## Core\n- [ ] Implement parser slice\n- [ ] Verify parser slice\n",
    )
    .unwrap();
    let first = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let first_source_tasks = load_lightweight_source_tasks(&run.run_id).unwrap();

    std::fs::write(
        &task_doc,
        "# Parser Work\n\n## Core\n- [ ] Verify parser slice\n- [ ] Implement parser slice\n",
    )
    .unwrap();
    let second = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let second_source_tasks = load_lightweight_source_tasks(&run.run_id).unwrap();

    let first_ids = first_source_tasks
        .iter()
        .map(|task| (task.title.clone(), task.source_task_id.clone()))
        .collect::<BTreeMap<_, _>>();
    let second_ids = second_source_tasks
        .iter()
        .map(|task| (task.title.clone(), task.source_task_id.clone()))
        .collect::<BTreeMap<_, _>>();

    assert_eq!(first.source_task_count, 2);
    assert_eq!(second.source_task_count, 2);
    assert_eq!(first_ids, second_ids);
}

#[test]
fn lightweight_broker_snapshot_parses_plain_bullets_and_numbered_items() {
    let fixture = CoordinationTestFixture::named("lightweight-source-task-mixed-lists");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );
    fixture.persist_run_with_tasks(&run, &[]);

    let task_doc = fixture
        .workspace
        .path()
        .join("docs/autonomous/lightweight-tasks-pass-2.md");
    std::fs::create_dir_all(task_doc.parent().unwrap()).unwrap();
    std::fs::write(
        &task_doc,
        "# Runtime Work\n\n## Tasks\n1. Wire namespace registry\n2. Validate namespace registry\n- Document follow-up gaps\n",
    )
    .unwrap();

    let snapshot = refresh_lightweight_broker_snapshot(&run.run_id).unwrap();
    let source_tasks = load_lightweight_source_tasks(&run.run_id).unwrap();

    assert_eq!(snapshot.source_task_count, 3);
    assert_eq!(
        source_tasks[0].source_task_id,
        "source-task-runtime-work--tasks--wire-namespace-registry"
    );
    assert_eq!(
        source_tasks[1].source_task_id,
        "source-task-runtime-work--tasks--validate-namespace-registry"
    );
    assert_eq!(
        source_tasks[2].source_task_id,
        "source-task-runtime-work--tasks--document-follow-up-gaps"
    );
    assert!(source_tasks.iter().all(|task| !task.completed));
}

#[test]
fn lightweight_next_task_doc_path_uses_pass_suffix_when_cycle_note_is_stale() {
    let fixture = CoordinationTestFixture::named("lightweight-next-task-doc-cycle-from-path");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks-pass-2.md",
        "codex",
        &["minimax".to_string()],
    );

    assert_eq!(
        lightweight_next_task_doc_path_for_run(&run),
        "docs/autonomous/lightweight-tasks-pass-3.md"
    );
}

#[test]
fn lightweight_claim_keeps_configured_provider_without_category_reroute() {
    let fixture = CoordinationTestFixture::named("lightweight-provider-claim");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["apply_patch".to_string(), "exec_command".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    fixture.persist_run_with_tasks(&run, &[task]);

    let claimed = claim_next_runnable_task(&run.run_id, 91, "codex")
        .unwrap()
        .expect("task should be claimed");
    assert_eq!(claimed.assigned_provider.as_deref(), Some("minimax"));

    let claim = load_claim(&run.run_id, &claimed.task_id).unwrap();
    assert_eq!(claim.provider, "minimax");
}

#[test]
fn lightweight_claim_fails_hard_when_configured_provider_is_unavailable() {
    let fixture = CoordinationTestFixture::named("lightweight-claim-unavailable");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    run.provider_cooldowns.insert(
        "minimax".to_string(),
        ProviderCooldown {
            unavailable_until: Utc::now() + chrono::Duration::minutes(30),
            reason: "MiniMax authentication failed.".to_string(),
        },
    );
    let task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["apply_patch".to_string(), "exec_command".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let claimed = claim_next_runnable_task(&run.run_id, 91, "codex").unwrap();
    assert!(claimed.is_none());

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.assigned_provider.as_deref(), Some("minimax"));
    assert!(failed
        .artifact_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("[provider_unavailable]")));

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(updated_run.status, CoordinationRunStatus::Failed);
}

#[test]
fn lightweight_review_advances_task_doc_without_materialized_tracking() {
    let notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut run = CoordinationRun {
        run_id: "lightweight-review-no-tracking".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };
    let review_task = CoordinationTask {
        task_id: "review".to_string(),
        run_id: run.run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("claude".to_string()),
        task_category: Some(TaskCategory::Review),
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Validate".to_string(),
        prompt: lightweight_review_prompt(&run),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["**".to_string()],
        write_scope: vec![
            "docs/autonomous/lightweight-tasks.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-1.md".to_string(),
        ],
        acceptance_checks: Vec::new(),
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
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("claude".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "Needs another pass".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "Needs another pass".to_string(),
            details: "Queued the next pass.".to_string(),
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Implement the next slice".to_string(),
                provider: None,
                prompt: "Do the next implementation slice.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                allowed_tools: vec!["apply_patch".to_string(), "exec_command".to_string()],
                acceptance_checks: vec![
                    "cargo check succeeds".to_string(),
                    "the slice is complete".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    let next_path =
        update_lightweight_task_doc_after_review(&mut run, &review_task, &artifact, &[]).unwrap();
    assert_eq!(next_path, "docs/autonomous/lightweight-tasks-pass-1.md");
}

struct BrokenWorkerReviewScenario {
    review_task_id: String,
    tasks: Vec<CoordinationTask>,
    snapshot: CoordinationRunSnapshot,
    events: Vec<CoordinationEvent>,
}

fn expand_broken_worker_review_scenario(
    fix_tasks: Vec<GeneratedTaskSpec>,
) -> BrokenWorkerReviewScenario {
    let fixture = CoordinationTestFixture::named("lightweight-broken-worker-review");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string(), "claude".to_string()],
    );

    let write_task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec![
            "src/lib.rs".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let validation_task = fixture.make_task_with_options(
        &run,
        "validation-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("gemini"),
        "Run validation",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string(), "Cargo.toml".to_string()],
        Vec::new(),
        vec![write_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    let review_task = fixture.make_task_with_options(
        &run,
        "review-task",
        None,
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        vec![
            "docs/autonomous/lightweight-tasks.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-1.md".to_string(),
        ],
        vec![validation_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(
        &run,
        &[
            write_task.clone(),
            validation_task.clone(),
            review_task.clone(),
        ],
    );

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: write_task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "worker changed src/lib.rs".to_string(),
        body: "introduced a todo! in src/lib.rs".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "worker changed src/lib.rs".to_string(),
            details: "introduced a todo! in src/lib.rs".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: validation_task.task_id.clone(),
        provider: Some("gemini".to_string()),
        task_category: Some(TaskCategory::Verify),
        fallback_from: None,
        summary: "cargo test failed".to_string(),
        body: "cargo test fails because src/lib.rs still contains todo!".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "cargo test failed".to_string(),
            details: "cargo test fails because src/lib.rs still contains todo!".to_string(),
            warnings: vec!["worker output broke the test run".to_string()],
            remaining_gaps: vec!["cargo test is red".to_string()],
            ..Default::default()
        },
    });

    let review_artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "repair pass required".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "repair pass required".to_string(),
            details: "Validation caught a broken worker change: cargo test fails after the src/lib.rs edit.".to_string(),
            blockers: vec!["cargo test fails after worker output".to_string()],
            remaining_gaps: vec!["src/lib.rs still contains broken code".to_string()],
            fix_tasks,
            ..Default::default()
        },
    };
    fixture.write_artifact(&review_artifact);

    expand_review_task(&run.run_id, &review_task, &review_artifact).unwrap();

    BrokenWorkerReviewScenario {
        tasks: load_tasks(&run.run_id).unwrap(),
        snapshot: load_run_snapshot(&run.run_id).expect("snapshot should load"),
        events: load_events(&run.run_id, None, None).unwrap_or_default(),
        review_task_id: review_task.task_id,
    }
}

fn broken_worker_fix_spec(title: &str, write_scope: Vec<String>) -> GeneratedTaskSpec {
    GeneratedTaskSpec {
        title: title.to_string(),
        provider: None,
        prompt:
            "Repair the broken worker output so tests pass again without editing the active task list."
                .to_string(),
        kind: "write".to_string(),
        estimated_minutes: 20,
        depends_on_titles: Vec::new(),
        read_scope: vec![
            "src/lib.rs".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        write_scope,
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec![
            "cargo test passes".to_string(),
            "review can confirm the repair without worker task-list edits".to_string(),
        ],
    }
}

#[test]
fn lightweight_review_queues_repair_work_when_validation_catches_broken_worker_code() {
    let scenario = expand_broken_worker_review_scenario(vec![broken_worker_fix_spec(
        "Fix broken src/lib.rs after failing validation",
        vec![
            "src/lib.rs".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
    )]);
    let repair_task = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedWriteTask
        })
        .unwrap();
    let follow_up_review = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
        })
        .unwrap();

    assert_eq!(repair_task.assigned_provider.as_deref(), Some("minimax"));
    assert!(repair_task
        .prompt
        .contains("The active task list document path for this run is"));
    assert!(repair_task.write_scope.contains(&"src/lib.rs".to_string()));
    assert_eq!(
        follow_up_review.dependencies,
        vec![repair_task.task_id.clone()]
    );
    assert_eq!(scenario.snapshot.run.status, CoordinationRunStatus::Active);
}

#[test]
fn lightweight_broken_worker_follow_up_review_waits_for_generated_repair() {
    let scenario = expand_broken_worker_review_scenario(vec![broken_worker_fix_spec(
        "Repair failed sample",
        vec!["src/lib.rs".to_string()],
    )]);
    let repair_task = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedWriteTask
        })
        .unwrap();
    let follow_up_review = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
        })
        .unwrap();

    assert_eq!(
        follow_up_review.dependencies,
        vec![repair_task.task_id.clone()]
    );
    assert_eq!(follow_up_review.status, CoordinationTaskStatus::Blocked);
}

#[test]
fn lightweight_broken_worker_review_emits_runtime_prefixed_repair_prompt() {
    let scenario = expand_broken_worker_review_scenario(vec![broken_worker_fix_spec(
        "Repair prompt prefix",
        vec!["src/lib.rs".to_string()],
    )]);
    let repair_task = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedWriteTask
        })
        .unwrap();

    assert!(repair_task
        .prompt
        .contains("The active task list document path for this run is"));
    assert!(repair_task
        .prompt
        .contains("Repair the broken worker output"));
}

#[test]
fn lightweight_broken_worker_review_rotates_worker_models_across_multiple_repairs() {
    let scenario = expand_broken_worker_review_scenario(vec![
        broken_worker_fix_spec("Fix src/lib.rs", vec!["src/lib.rs".to_string()]),
        broken_worker_fix_spec(
            "Fix docs/autonomous/lightweight-tasks.md",
            vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        ),
    ]);
    let generated_writes = scenario
        .tasks
        .iter()
        .filter(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedWriteTask
        })
        .collect::<Vec<_>>();
    let mut generated_writes = generated_writes;
    generated_writes.sort_by(|left, right| left.title.cmp(&right.title));

    assert_eq!(generated_writes.len(), 2);
    assert_eq!(
        generated_writes[0].title,
        "Fix docs/autonomous/lightweight-tasks.md"
    );
    assert_eq!(
        generated_writes[0].assigned_provider.as_deref(),
        Some("claude")
    );
    assert_eq!(generated_writes[1].title, "Fix src/lib.rs");
    assert_eq!(
        generated_writes[1].assigned_provider.as_deref(),
        Some("minimax")
    );
}

#[test]
fn lightweight_broken_worker_follow_up_review_depends_on_all_generated_repairs() {
    let scenario = expand_broken_worker_review_scenario(vec![
        broken_worker_fix_spec("Repair src/lib.rs", vec!["src/lib.rs".to_string()]),
        broken_worker_fix_spec(
            "Repair task list state",
            vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        ),
    ]);
    let generated_write_ids = scenario
        .tasks
        .iter()
        .filter(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedWriteTask
        })
        .map(|task| task.task_id.clone())
        .collect::<Vec<_>>();
    let mut generated_write_ids = generated_write_ids;
    generated_write_ids.sort();
    let follow_up_review = scenario
        .tasks
        .iter()
        .find(|task| {
            task.parent_task_id.as_deref() == Some(scenario.review_task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
        })
        .unwrap();
    let mut follow_up_dependencies = follow_up_review.dependencies.clone();
    follow_up_dependencies.sort();

    assert_eq!(follow_up_dependencies, generated_write_ids);
}

#[test]
fn lightweight_broken_worker_review_records_fix_loop_event() {
    let scenario = expand_broken_worker_review_scenario(vec![broken_worker_fix_spec(
        "Repair failure event",
        vec!["src/lib.rs".to_string()],
    )]);
    assert!(scenario.events.iter().any(|event| {
        event.event_type == "review_fix_loop"
            && event.message.contains("queued 1 follow-up task(s)")
    }));
}

fn init_lightweight_git_repo(workspace: &std::path::Path) {
    use std::process::Command;
    fn run_git(workspace: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(workspace)
            .output()
            .unwrap();
        assert!(output.status.success(), "git {:?} should succeed", args);
    }
    run_git(workspace, &["init", "-b", "main"]);
    run_git(workspace, &["config", "user.name", "Test User"]);
    run_git(workspace, &["config", "user.email", "test@example.com"]);
    run_git(workspace, &["add", "src/lib.rs"]);
    run_git(workspace, &["commit", "-m", "initial"]);
    // Create bare remote and add as origin
    let remote_path = workspace.join(".test-remote.git");
    std::fs::create_dir_all(&remote_path).unwrap();
    run_git(&remote_path, &["init", "--bare"]);
    run_git(
        workspace,
        &["remote", "add", "origin", &remote_path.to_string_lossy()],
    );
    run_git(workspace, &["push", "-u", "origin", "main"]);
}

#[test]
fn lightweight_completion_skips_acceptance_and_materialized_tracking() {
    let fixture = CoordinationTestFixture::named("lightweight-complete");
    init_lightweight_git_repo(fixture.workspace.path());
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.auto_retry_failed_tasks = false;
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["apply_patch".to_string(), "exec_command".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    task.claimed_by = Some(71);
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 71,
        launcher_pid: Some(71),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { todo!() }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: Some("minimax".to_string()),
            task_category: Some(TaskCategory::DeepWrite),
            fallback_from: None,
            summary: "implemented slice".to_string(),
            body: "updated src/lib.rs".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedWithChanges,
                summary: "implemented slice".to_string(),
                details: "updated src/lib.rs".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                ..Default::default()
            },
        },
    )
    .unwrap();

    let persisted = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Completed);
    assert!(!artifact_materialized_files_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn lightweight_review_does_not_open_review_approval_interrupt() {
    let fixture = CoordinationTestFixture::named("lightweight-review-no-approval");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.require_review_before_apply = true;
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate lightweight implementation pass",
        &["docs/autonomous/lightweight-tasks.md"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&review_task));

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "Need another pass".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "Need another pass".to_string(),
            details: "Queued follow-up work.".to_string(),
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Follow-up write".to_string(),
                provider: None,
                prompt: "Implement the next fix.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["docs/autonomous/lightweight-tasks.md".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                allowed_tools: vec!["functions.apply_patch".to_string()],
                acceptance_checks: vec!["follow-up work is complete".to_string()],
            }],
            ..Default::default()
        },
    };

    let interrupt = maybe_create_review_approval_interrupt(&run.run_id, &review_task, &artifact)
        .expect("interrupt creation should succeed");
    assert!(interrupt.is_none());
    assert!(!has_open_interrupts(&run.run_id));
}

#[test]
fn lightweight_provider_cooldown_fails_without_fallback_or_retry() {
    let fixture = CoordinationTestFixture::named("lightweight-cooldown");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["apply_patch".to_string(), "exec_command".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(81);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 81,
        launcher_pid: Some(81),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    assert!(requeue_task_for_provider_cooldown(
        &run.run_id,
        &task.task_id,
        81,
        "minimax",
        "failed to authenticate"
    )
    .unwrap());

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.assigned_provider.as_deref(), Some("minimax"));
    assert!(failed
        .artifact_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("[provider_unavailable]")));
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(updated_run.status, CoordinationRunStatus::Failed);
}

#[test]
fn lightweight_run_form_defaults_task_doc_from_design_doc_when_note_is_missing() {
    let run = CoordinationRun {
        run_id: "lightweight-form".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/autonomous/lightweight-implementation-doc-abcd1234.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: lightweight_notes_with_settings("", "", "codex", &["codex".to_string()]),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    assert_eq!(
        lightweight_current_task_doc_path(&run),
        "docs/autonomous/lightweight-implementation-doc-abcd1234-tasks.md"
    );
}

#[test]
fn lightweight_repo_configuration_round_trips_rust_selection() {
    let notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let run = CoordinationRun {
        run_id: "lightweight-rust-config".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::LightweightImplementation,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    assert_eq!(
        lightweight_repo_configuration_for_run(&run),
        LightweightRepoConfiguration::Rust
    );
}

#[test]
fn validate_lightweight_notes_rejects_unknown_repo_configuration() {
    let notes = [
        "lightweight_task_doc=docs/autonomous/lightweight-tasks.md",
        "lightweight_task_splitter=codex",
        "lightweight_worker_models=codex",
        "lightweight_repo_configuration=python",
    ]
    .join("\n");

    let error = validate_lightweight_notes(&notes, "docs/design.md")
        .expect_err("unknown repo configuration should fail validation");
    assert!(error
        .to_string()
        .contains("unknown lightweight repo configuration"));
}

#[test]
fn lightweight_follow_up_review_recomputes_scope_from_current_task_doc() {
    let fixture = CoordinationTestFixture::named("lightweight-follow-up-scope");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = [
        format!("{LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY}=codex"),
        format!("{LIGHTWEIGHT_WORKERS_NOTE_KEY}=minimax"),
        format!("{LIGHTWEIGHT_TASK_DOC_NOTE_KEY}=docs/autonomous/lightweight-tasks-pass-2.md"),
        format!("{LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY}=2"),
    ]
    .join("\n");
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate lightweight implementation pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[
            "docs/autonomous/lightweight-tasks.md",
            "docs/autonomous/lightweight-tasks-pass-1.md",
        ],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[review_task.clone()]);

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "needs one more pass".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "needs one more pass".to_string(),
            details: "follow-up tasks required".to_string(),
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Refresh docs".to_string(),
                provider: None,
                prompt: "Refresh the coordination docs.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["docs/autonomous".to_string()],
                write_scope: vec!["docs/autonomous/review-notes.md".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "docs are updated".to_string(),
                    "review notes reflect the current state".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let follow_up = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .unwrap();
    assert_eq!(
        follow_up.write_scope,
        vec![
            "docs/design.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-2.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-3.md".to_string(),
        ]
    );
    assert!(follow_up.prompt.contains("docs/design.md"));
    assert!(follow_up
        .prompt
        .contains("docs/autonomous/lightweight-tasks-pass-2.md"));
    assert!(follow_up
        .prompt
        .contains("docs/autonomous/lightweight-tasks-pass-3.md"));
}

#[test]
fn lightweight_review_read_only_fix_tasks_do_not_advance_pass_doc() {
    let fixture = CoordinationTestFixture::named("lightweight-read-only-pass-stop");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = [
        format!("{LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY}=codex"),
        format!("{LIGHTWEIGHT_WORKERS_NOTE_KEY}=minimax"),
        format!("{LIGHTWEIGHT_TASK_DOC_NOTE_KEY}=docs/autonomous/lightweight-tasks-pass-2.md"),
        format!("{LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY}=2"),
    ]
    .join("\n");
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate lightweight implementation pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[
            "docs/design.md",
            "docs/autonomous/lightweight-tasks-pass-2.md",
            "docs/autonomous/lightweight-tasks-pass-3.md",
        ],
        &[],
        CoordinationTaskStatus::Completed,
    );

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "needs environment validation".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "needs environment validation".to_string(),
            details: "the next step is rerunning proof in a valid external environment".to_string(),
            remaining_gaps: vec![
                "re-run canonical proof in a valid supported-zig environment".to_string(),
            ],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Re-run canonical proof in valid environment".to_string(),
                provider: None,
                prompt: "Re-run validation once the external Zig environment is fixed.".to_string(),
                kind: "read".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: Vec::new(),
                allowed_tools: vec!["filesystem".to_string(), "search".to_string()],
                acceptance_checks: vec!["the proof is rerun in a valid environment".to_string()],
            }],
            ..Default::default()
        },
    };

    let updated = update_lightweight_task_doc_after_review(&mut run, &review_task, &artifact, &[]);
    assert!(updated.is_none());
    assert!(run
        .notes
        .contains("lightweight_task_doc=docs/autonomous/lightweight-tasks-pass-2.md"));
    assert!(run.notes.contains("lightweight_task_doc_cycle=2"));
}

#[test]
fn lightweight_rust_plan_prompt_requires_scaffold_alignment_for_existing_workspace() {
    let fixture = CoordinationTestFixture::named("lightweight-rust-plan-guidance");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );

    let tasks = seed_initial_tasks(&run);
    assert_eq!(tasks.len(), 5);
    let conversion_task = tasks
        .iter()
        .find(|task| task.title == "Convert scaffolded Rust workspace to current repo")
        .unwrap();
    assert_eq!(
        conversion_task.kind,
        CoordinationTaskKind::GeneratedWriteTask
    );
    assert_eq!(conversion_task.task_category, Some(TaskCategory::DeepWrite));
    assert!(conversion_task
        .prompt
        .contains("Convert the scaffolded repository so it belongs to this repo"));
    assert!(conversion_task
        .prompt
        .contains("Update scaffold names, workspace members, package identifiers"));
    assert!(conversion_task
        .allowed_tools
        .contains(&"sequentialthinking".to_string()));

    let implementation_plan = tasks
        .iter()
        .find(|task| task.title == "Split lightweight implementation tasks")
        .unwrap();
    assert_eq!(
        implementation_plan.dependencies,
        vec![conversion_task.task_id.clone()]
    );
    assert!(
        implementation_plan
            .prompt
            .contains("$AUTO_RUNS_RUST_SCAFFOLD_ROOT")
            || implementation_plan
                .prompt
                .contains(&super::workspace_ready::rust_scaffold_root())
    );
    assert!(implementation_plan
        .prompt
        .contains("If the workspace is fresh or only contains `docs/`"));
    assert!(implementation_plan
        .prompt
        .contains("dedicated LLM conversion step"));
    assert!(implementation_plan
        .prompt
        .contains("the runtime seeds the full scaffold tree except `.git` and `target`"));
    assert!(implementation_plan
        .prompt
        .contains("rename copied scaffold references for the current repository"));
}

#[test]
fn provider_unavailable_failure_does_not_schedule_repository_self_heal() {
    let fixture = CoordinationTestFixture::named("provider-outage-is-not-a-code-repair");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut task = fixture.make_task_with_options(
        &run,
        "unavailable-provider",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["apply_patch".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Failed,
    );
    task.artifact_summary = Some("[provider_unavailable] authentication required".to_string());
    task.failure_kind = Some(TaskFailureKind::ProviderUnavailable);
    fixture.persist_run_with_tasks(&run, &[task]);
    assert!(!maybe_auto_fix_failed_run(&run.run_id.clone(), &mut run).unwrap());
    assert_eq!(auto_fix_failed_run_restarts_used(&run), 0);
}
