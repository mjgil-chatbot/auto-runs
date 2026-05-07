#[test]
fn draft_design_contract_explicitly_wraps_markdown_in_details() {
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::DraftDesignDoc,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Draft".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::DraftDesignDoc,
        )),
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let contract = provider_response_contract(&task);
    assert!(contract.contains("typed task artifact envelope"));
    assert!(contract.contains("`payload_kind` must be `design_doc`"));
    assert!(contract.contains("Put the full markdown design proposal in `payload.markdown`"));
    assert!(contract.contains("validate_autonomous_task_report.py"));
    assert!(contract.contains("--workspace-root ."));
}

#[test]
fn plan_contract_includes_plan_validator() {
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::PlanFlatTasks,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Plan".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::PlanFlatTasks,
        )),
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let contract = provider_response_contract(&task);
    assert!(contract.contains("`payload_kind` must be `plan_tasks`"));
    assert!(contract.contains("`payload.generated_tasks`"));
    assert!(contract.contains("`estimated_minutes`"));
    assert!(contract.contains("`depends_on_titles`"));
    assert!(contract.contains("--kind plan"));
    assert!(!contract.contains("--workspace-root"));
}

#[test]
fn mutating_plan_contract_omits_changed_files_and_keeps_workspace_validator() {
    let fixture = CoordinationTestFixture::named("lightweight-plan-contract");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task_with_options(
        &run,
        "task",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Bootstrap lightweight plan",
        vec![
            "filesystem".to_string(),
            "search".to_string(),
            "edit".to_string(),
        ],
        vec!["**".to_string()],
        vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("set `outcome` to `completed_with_changes`"));
    assert!(contract.contains("use `completed_no_change`"));
    assert!(contract.contains("Do not include a `changed_files` field"));
    assert!(contract.contains("`estimated_minutes`"));
    assert!(contract.contains("`depends_on_titles`"));
    assert!(contract.contains("set `read_scope` to `[\"**\"]`"));
    assert!(contract.contains("set `write_scope` to `[\"**\"]`"));
    assert!(contract.contains("patch-level git publish is the consolidation boundary"));
    assert!(contract.contains("--kind plan"));
    assert!(contract.contains(&format!("--workspace-root {}", run.workspace_path)));
    assert!(contract.contains("--allow-missing-changed-files"));
}

#[test]
fn lightweight_generated_write_contract_uses_plaintext_without_validator_hint() {
    let fixture = CoordinationTestFixture::named("lightweight-write-contract");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "lightweight-write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Create static viewer shell",
        &["docs/design.md"],
        &["index.html", "src/styles.css"],
        &["shell renders", "styles load"],
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("Return concise plain text only"));
    assert!(contract.contains("Do not return a task-report JSON object"));
    assert!(contract.contains("COMPLETED:"));
    assert!(!contract.contains("validate_autonomous_task_report.py"));
    assert!(!contract.contains("--workspace-root"));
    assert!(!contract.contains("changed_files"));
}

#[test]
fn lightweight_generated_read_contract_uses_plaintext_without_validator_hint() {
    let fixture = CoordinationTestFixture::named("lightweight-read-contract");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "lightweight-read",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Validate browser shell",
        &["index.html", "src"],
        &[],
        &["validation evidence recorded"],
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("Return concise plain text only"));
    assert!(contract.contains("Do not return a task-report JSON object"));
    assert!(contract.contains("NO_CHANGE:"));
    assert!(!contract.contains("validate_autonomous_task_report.py"));
    assert!(!contract.contains("--workspace-root"));
    assert!(!contract.contains("changed_files"));
}

#[test]
fn native_structured_output_override_attaches_contract_schema() {
    let contract = lookup_contract(CoordinationTaskKind::DraftDesignDoc)
        .expect("draft design doc should have a typed contract");
    let mut overrides = crate::provider_bridge::ProviderRequestOverrides::default();

    let mode = apply_structured_output_overrides("claude", contract, &mut overrides)
        .expect("structured output override should build");

    assert_eq!(mode, StructuredOutputMode::ProviderNativeJsonSchema);
    let schema: serde_json::Value = serde_json::from_str(
        overrides
            .output_json_schema
            .as_deref()
            .expect("native schema should be attached"),
    )
    .expect("schema should parse");
    assert_eq!(
        schema
            .pointer("/properties/payload_kind/const")
            .and_then(serde_json::Value::as_str),
        Some("design_doc")
    );
    assert_eq!(
        schema
            .pointer("/properties/contract_version/const")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    let payload_markdown_type = schema
        .pointer("/properties/payload/properties/markdown/type")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            schema
                .pointer("/properties/payload/$ref")
                .and_then(serde_json::Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .and_then(|definition| {
                    schema
                        .pointer(&format!("/$defs/{definition}/properties/markdown/type"))
                        .and_then(serde_json::Value::as_str)
                })
        })
        .or_else(|| {
            schema
                .pointer("/properties/payload/$ref")
                .and_then(serde_json::Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/definitions/"))
                .and_then(|definition| {
                    schema
                        .pointer(&format!(
                            "/definitions/{definition}/properties/markdown/type"
                        ))
                        .and_then(serde_json::Value::as_str)
                })
        });
    assert_eq!(payload_markdown_type, Some("string"));
}

#[test]
fn regex_structured_output_falls_back_for_embedded_markdown_contracts() {
    let contract = lookup_contract(CoordinationTaskKind::DraftDesignDoc)
        .expect("draft design doc should have a typed contract");
    let provider = ProviderCapabilityProfile {
        provider_id: "regex-test".to_string(),
        structured_output_mode: StructuredOutputMode::RegexConstrainedJson,
        ..Default::default()
    };

    assert_eq!(
        select_structured_output_mode(&provider, contract),
        StructuredOutputMode::PromptOnlyFallback
    );
}

#[test]
fn generated_write_contract_requires_full_document_in_details() {
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Write prompt artifact".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: vec!["docs/design.md".to_string()],
        write_scope: vec!["docs/autonomous/prompts/track-1.md".to_string()],
        acceptance_checks: vec!["file exists".to_string()],
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::GeneratedWriteTask,
        )),
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let contract = provider_response_contract(&task);
    assert!(contract.contains("full final contents of every changed document/text file"));
    assert!(contract.contains("put the complete final file contents directly in `details`"));
}

#[test]
fn review_contract_uses_typed_findings_payload_without_review_metadata() {
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Review".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
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

    let contract = provider_response_contract(&task);
    assert!(contract.contains("typed task artifact envelope"));
    assert!(contract.contains("`payload_kind` must be `review_findings`"));
    assert!(contract.contains("`payload.findings`"));
    assert!(contract.contains("`payload.fix_tasks`"));
    assert!(contract.contains("validate_autonomous_task_report.py"));
}

#[test]
fn code_review_contract_requires_findings_when_metadata_exists() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let run_id = format!("review-contract-{}", Uuid::new_v4());
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Review".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
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
    write_review_task_metadata(
        &run_id,
        &task.task_id,
        &code_review::ReviewTaskMetadata {
            scope_description: "Current workspace".to_string(),
            dimensions: vec!["architecture".to_string()],
            chunk: vec![code_review::ReviewChunkTarget {
                path: "src/lib.rs".to_string(),
                start_line: 1,
                end_line: 20,
            }],
            automated_checks: Vec::new(),
            profile_path: code_review::REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: Some(ReviewFixMode::ReportOnly),
            output_path: None,
        },
    )
    .expect("metadata should write");

    let contract = provider_response_contract(&task);
    assert!(contract.contains("`payload.findings`"));
    assert!(contract.contains("report-only review mode"));

    let _ = fs::remove_dir_all(run_dir(&run_id));
}

#[test]
fn code_review_seed_uses_plan_then_materialize_pipeline() {
    let run = CoordinationRun {
        run_id: "review-run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
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
    assert_eq!(tasks.len(), 3);
    assert_eq!(tasks[0].kind, CoordinationTaskKind::SurveyWorkspace);
    assert_eq!(tasks[1].kind, CoordinationTaskKind::PlanCodeReview);
    assert_eq!(tasks[2].kind, CoordinationTaskKind::MaterializeDesignDoc);
    assert_eq!(tasks[2].write_scope, vec![run.design_doc_path.clone()]);
}

#[test]
fn upsert_run_persists_code_review_configuration() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let workspace =
        std::env::temp_dir().join(format!("rust-chatbot-review-run-{}", Uuid::new_v4()));
    fs::create_dir_all(&workspace).unwrap();

    let created = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "review the workspace".to_string(),
        workflow_mode: AutonomousWorkflowMode::CodeReview,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: "initial review".to_string(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: Some(ReviewScope::Diff {
            base: "main".to_string(),
        }),
        review_dimensions: Some(vec!["architecture".to_string(), "code_quality".to_string()]),
        review_fix_mode: Some(ReviewFixMode::ReportOnly),
    })
    .expect("create run should succeed");

    assert_eq!(
        created.run.review_scope,
        Some(ReviewScope::Diff {
            base: "main".to_string()
        })
    );
    assert_eq!(
        created.run.review_dimensions,
        Some(vec!["architecture".to_string(), "code_quality".to_string(),])
    );
    assert_eq!(created.run.review_fix_mode, Some(ReviewFixMode::ReportOnly));

    // Verify the seeded run rejects configuration edits.
    let run_id = created.run.run_id.clone();
    let update_err = upsert_run(UpsertRunInput {
        existing_run_id: Some(run_id.clone()),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "review the workspace".to_string(),
        workflow_mode: AutonomousWorkflowMode::CodeReview,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: "updated review".to_string(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: Some(ReviewScope::Globs(vec!["src/**/*.rs".to_string()])),
        review_dimensions: Some(vec!["test_coverage".to_string()]),
        review_fix_mode: Some(ReviewFixMode::GenerateFixes),
    })
    .expect_err("updating a seeded run should fail");
    assert!(
        format!("{update_err}").contains("tasks have already been seeded"),
        "unexpected error: {update_err}"
    );

    // Verify the original config survives the rejected update and reloads correctly.
    let reloaded = load_run(&run_id).expect("run should reload");
    assert_eq!(
        reloaded.review_scope,
        Some(ReviewScope::Diff {
            base: "main".to_string()
        })
    );
    assert_eq!(
        reloaded.review_dimensions,
        Some(vec!["architecture".to_string(), "code_quality".to_string()])
    );
    assert_eq!(reloaded.review_fix_mode, Some(ReviewFixMode::ReportOnly));

    let _ = fs::remove_dir_all(run_dir(&run_id));
    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn upsert_run_generates_workflow_derived_hashed_design_doc_path_for_defaults() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let workspace = std::env::temp_dir().join(format!(
        "rust-chatbot-default-design-doc-{}",
        Uuid::new_v4()
    ));
    fs::create_dir_all(&workspace).unwrap();

    let created = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::DesignImplementation,
        design_doc_path: crate::default_design_doc_path_for_workflow(
            &AutonomousWorkflowMode::DesignImplementation,
        ),
        completion_criteria: String::new(),
        max_iterations: 1,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("create run should succeed");

    assert_eq!(
        created.run.design_doc_path,
        crate::default_design_doc_path_for_run(
            &AutonomousWorkflowMode::DesignImplementation,
            &created.run.run_id,
        )
    );
}

#[test]
fn rewire_follow_up_tasks_only_rewrites_real_downstream_dependents() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = EnvVarGuard::set("HOME", _home_dir.path());
    let run_id = format!("rewire-follow-up-{}", Uuid::new_v4());
    let now = Utc::now();
    let source_task_id = "plan-task";

    let generated_review = CoordinationTask {
        task_id: "generated-review".to_string(),
        run_id: run_id.clone(),
        parent_task_id: Some(source_task_id.to_string()),
        kind: CoordinationTaskKind::ReviewArtifacts,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Generated review".to_string(),
        prompt: "Review prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string()],
        read_scope: vec!["src/**".to_string()],
        write_scope: Vec::new(),
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: vec![source_task_id.to_string()],
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
        created_at: now,
        updated_at: now,
    };
    let downstream_materialize = CoordinationTask {
        task_id: "materialize".to_string(),
        run_id: run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::MaterializeDesignDoc,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Materialize".to_string(),
        prompt: "Write doc".to_string(),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["artifacts/**".to_string()],
        write_scope: vec!["docs/code-review.md".to_string()],
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: vec![source_task_id.to_string()],
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::MaterializeDesignDoc,
        )),
        status: CoordinationTaskStatus::Queued,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };
    let unrelated_materialize = CoordinationTask {
        task_id: "canonical-doc".to_string(),
        run_id: run_id.clone(),
        parent_task_id: None,
        kind: CoordinationTaskKind::MaterializeDesignDoc,
        assigned_provider: Some("claude".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Canonical doc".to_string(),
        prompt: "Write canonical doc".to_string(),
        cwd: ".".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["artifacts/**".to_string()],
        write_scope: vec!["docs/design.md".to_string()],
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: vec!["other-task".to_string()],
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::MaterializeDesignDoc,
        )),
        status: CoordinationTaskStatus::Completed,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    let mut tasks = vec![
        generated_review.clone(),
        downstream_materialize.clone(),
        unrelated_materialize.clone(),
    ];

    for task in &tasks {
        write_json_atomic(&task_path(&run_id, &task.task_id), task).unwrap();
    }

    let generated_ids = vec!["generated-review".to_string(), "generated-fix".to_string()];
    rewire_follow_up_tasks(&run_id, source_task_id, &mut tasks, &generated_ids).unwrap();

    assert_eq!(tasks[0].dependencies, vec![source_task_id.to_string()]);
    assert_eq!(tasks[0].status, CoordinationTaskStatus::Queued);

    assert_eq!(tasks[1].dependencies, generated_ids);
    assert_eq!(tasks[1].status, CoordinationTaskStatus::Blocked);

    assert_eq!(tasks[2].dependencies, vec!["other-task".to_string()]);
    assert_eq!(tasks[2].status, CoordinationTaskStatus::Completed);

    let _ = fs::remove_dir_all(run_dir(&run_id));
}

#[test]
fn review_schema_requires_all_properties_in_required() {
    let contract = lookup_contract(CoordinationTaskKind::ReviewArtifacts)
        .expect("review contract should exist");
    let mut overrides = crate::provider_bridge::ProviderRequestOverrides::default();
    let mode = apply_structured_output_overrides("codex", contract, &mut overrides)
        .expect("structured output override should build");
    assert_eq!(mode, StructuredOutputMode::ProviderNativeJsonSchema);

    let schema: serde_json::Value = serde_json::from_str(
        overrides
            .output_json_schema
            .as_deref()
            .expect("native schema should be attached"),
    )
    .expect("schema should parse");

    let payload_def = schema
        .pointer("/$defs/ReviewFindingsPayload")
        .or_else(|| schema.pointer("/definitions/ReviewFindingsPayload"))
        .expect("ReviewFindingsPayload definition should exist");

    let props = payload_def
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("payload should have properties");
    let required = payload_def
        .get("required")
        .and_then(|v| v.as_array())
        .expect("payload should have required array");
    let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

    for key in props.keys() {
        assert!(
            required_strs.contains(&key.as_str()),
            "property '{}' missing from required array in ReviewFindingsPayload",
            key
        );
    }
}

#[test]
fn review_finding_schema_requires_all_properties() {
    let contract = lookup_contract(CoordinationTaskKind::ReviewArtifacts)
        .expect("review contract should exist");
    let mut overrides = crate::provider_bridge::ProviderRequestOverrides::default();
    apply_structured_output_overrides("codex", contract, &mut overrides)
        .expect("structured output override should build");

    let schema: serde_json::Value = serde_json::from_str(
        overrides
            .output_json_schema
            .as_deref()
            .expect("schema should be attached"),
    )
    .expect("schema should parse");

    let finding_def = schema
        .pointer("/$defs/ReviewFinding")
        .or_else(|| schema.pointer("/definitions/ReviewFinding"))
        .expect("ReviewFinding definition should exist");

    let props = finding_def
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("finding should have properties");
    let required = finding_def
        .get("required")
        .and_then(|v| v.as_array())
        .expect("finding should have required array");
    let required_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();

    for key in props.keys() {
        assert!(
            required_strs.contains(&key.as_str()),
            "property '{}' missing from required array in ReviewFinding",
            key
        );
    }
}
