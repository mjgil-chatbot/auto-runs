#[test]
fn lightweight_repo_configuration_round_trips_zig_selection() {
    let notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Zig,
    );
    let run = CoordinationRun {
        run_id: "lightweight-zig-config".to_string(),
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
        LightweightRepoConfiguration::Zig
    );
}

#[test]
fn lightweight_current_task_doc_path_normalizes_workspace_absolute_note() {
    let workspace = "/tmp/lightweight-zig";
    let run = CoordinationRun {
        run_id: "lightweight-absolute-task-doc".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string(),
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
        notes: lightweight_notes_with_settings(
            "",
            "/tmp/lightweight-zig/docs/task-list.md",
            "codex",
            &["codex".to_string()],
        ),
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
        "docs/task-list.md".to_string()
    );
    assert_eq!(
        lightweight_next_task_doc_path_for_run(&run),
        "docs/task-list-pass-1.md".to_string()
    );
}
