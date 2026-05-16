    #[test]
    fn review_findings_schema_rejects_missing_line_numbers() {
        let err = code_review::parse_review_response(
            "{\"outcome\":\"completed_no_change\",\"summary\":\"reviewed\",\"details\":\"done\",\"changed_files\":[],\"tests_run\":[],\"warnings\":[],\"blockers\":[],\"remaining_gaps\":[],\"design_requirements_satisfied\":[\"checked\"],\"fix_tasks\":[],\"findings\":[{\"severity\":\"high\",\"confidence\":\"high\",\"file\":\"src/lib.rs\",\"line\":0,\"dimension\":\"architecture\",\"description\":\"bad\",\"fix_suggestion\":\"fix it\"}]}",
        )
        .expect_err("invalid findings should be rejected");

        assert!(err.to_string().contains("line must be > 0"));
    }

    #[test]
    fn ralph_plan_prompt_requires_full_acceptance_criteria_tracking() {
        let run = CoordinationRun {
            run_id: "run".to_string(),
            root_chat_id: "chat".to_string(),
            workspace_path: ".".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "ship the change".to_string(),
            workflow_mode: AutonomousWorkflowMode::RalphLoop,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: "- acceptance A\n- acceptance B".to_string(),
            max_iterations: 2,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: "check the repo state first".to_string(),
            council_turn_policy: None,
            council_max_turns: None,
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            provider_cooldowns: BTreeMap::new(),
            task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
        };

        let prompt = build_ralph_plan_prompt(&run, 1);
        assert!(prompt.contains("Create the initial Ralph working plan"));
        assert!(prompt.contains("Raw Acceptance Criteria"));
        assert!(prompt.contains("Status Board"));
        assert!(prompt.contains("copy every completion criterion"));
        assert!(prompt.contains("acceptance A"));
        assert!(prompt.contains("acceptance B"));
    }

    #[test]
    fn freeform_council_seed_returns_no_dag_tasks() {
        let run = CoordinationRun {
            run_id: "run-council".to_string(),
            root_chat_id: "chat".to_string(),
            workspace_path: ".".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "claude".to_string(),
            goal: "build a chatbot".to_string(),
            workflow_mode: AutonomousWorkflowMode::FreeformCouncil,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: "- all tests pass\n- feature works end to end".to_string(),
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
            council_turn_policy: None,
            council_max_turns: None,
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            provider_cooldowns: BTreeMap::new(),
            task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
        };

        let tasks = seed_initial_tasks(&run);
        assert!(tasks.is_empty(), "FreeformCouncil must not seed DAG tasks");
    }

    #[test]
    fn freeform_council_does_not_regress_other_workflow_seeds() {
        let base = CoordinationRun {
            run_id: "run".to_string(),
            root_chat_id: "chat".to_string(),
            workspace_path: ".".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "goal".to_string(),
            workflow_mode: AutonomousWorkflowMode::SingleProvider,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
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

        let single = seed_initial_tasks(&base);
        assert!(
            !single.is_empty(),
            "SingleProvider must still seed DAG tasks"
        );

        let ralph = seed_initial_tasks(&CoordinationRun {
            workflow_mode: AutonomousWorkflowMode::RalphLoop,
            completion_criteria: "- criterion".to_string(),
            ..base.clone()
        });
        assert!(!ralph.is_empty(), "RalphLoop must still seed DAG tasks");

        let design_impl = seed_initial_tasks(&CoordinationRun {
            workflow_mode: AutonomousWorkflowMode::DesignImplementation,
            max_agents: 4,
            ..base.clone()
        });
        assert!(
            !design_impl.is_empty(),
            "DesignImplementation must seed DAG tasks"
        );

        let design_consensus = seed_initial_tasks(&CoordinationRun {
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            max_agents: 4,
            ..base.clone()
        });
        assert!(
            !design_consensus.is_empty(),
            "DesignConsensus must seed DAG tasks"
        );
        // DesignConsensus stops at the doc — must NOT contain PlanFlatTasks
        let dc_kinds: Vec<_> = design_consensus.iter().map(|t| &t.kind).collect();
        assert!(
            !dc_kinds.contains(&&CoordinationTaskKind::PlanFlatTasks),
            "DesignConsensus must not seed PlanFlatTasks"
        );
        // DesignImplementation must contain PlanFlatTasks
        let di_kinds: Vec<_> = design_impl.iter().map(|t| &t.kind).collect();
        assert!(
            di_kinds.contains(&&CoordinationTaskKind::PlanFlatTasks),
            "DesignImplementation must seed PlanFlatTasks"
        );
    }

    #[test]
    fn council_json_and_ledger_paths_are_inside_run_dir() {
        let _env_lock = crate::TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let run_id = "test-council-run";
        let cpath = council_json_path(run_id);
        let lpath = council_ledger_path(run_id);

        assert!(cpath.ends_with("council.json"));
        assert!(lpath.ends_with("ledger.jsonl"));
        assert_eq!(cpath.parent(), lpath.parent());
        assert_eq!(cpath.parent().unwrap(), run_dir(run_id).as_path());
    }

    #[test]
    fn derive_run_status_returns_planning_for_empty_tasks() {
        // Council runs must skip derive_run_status (returns Planning for empty task lists).
        let status = derive_run_status(&[], &[]);
        assert_eq!(status, CoordinationRunStatus::Planning);
    }

    #[test]
    fn derive_run_status_ignores_recovery_metadata_on_completed_tasks() {
        let now = Utc::now();
        let repaired_task = CoordinationTask {
            task_id: "repaired".to_string(),
            run_id: "run-1".to_string(),
            parent_task_id: None,
            kind: CoordinationTaskKind::GeneratedWriteTask,
            assigned_provider: Some("codex".to_string()),
            task_category: None,
            provider_hint: None,
            required_role: None,
            iteration: None,
            title: "Previously repaired".to_string(),
            prompt: "noop".to_string(),
            cwd: ".".to_string(),
            allowed_tools: Vec::new(),
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            acceptance_checks: Vec::new(),
            review_metadata: None,
            dependencies: Vec::new(),
            execution_generation: default_execution_generation(),
            dependency_generation: None,
            attempt_count: 1,
            max_attempts: 3,
            publish_attempt_count: 0,
            estimated_minutes: None,
            status: CoordinationTaskStatus::Completed,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: Some(TaskFailureKind::DependencyDrift),
            recovery_action: Some(TaskRecoveryAction::ReconcileState),
            created_at: now,
            updated_at: now,
        };
        let active_task = CoordinationTask {
            task_id: "active".to_string(),
            run_id: "run-1".to_string(),
            parent_task_id: None,
            kind: CoordinationTaskKind::GeneratedWriteTask,
            assigned_provider: Some("codex".to_string()),
            task_category: None,
            provider_hint: None,
            required_role: None,
            iteration: None,
            title: "Still running".to_string(),
            prompt: "noop".to_string(),
            cwd: ".".to_string(),
            allowed_tools: Vec::new(),
            read_scope: Vec::new(),
            write_scope: Vec::new(),
            acceptance_checks: Vec::new(),
            review_metadata: None,
            dependencies: Vec::new(),
            execution_generation: default_execution_generation(),
            dependency_generation: None,
            attempt_count: 1,
            max_attempts: 3,
            publish_attempt_count: 0,
            estimated_minutes: None,
            status: CoordinationTaskStatus::Running,
            claimed_by: Some(1),
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: now,
            updated_at: now,
        };

        let status = derive_run_status(&[repaired_task, active_task], &[]);
        assert_eq!(status, CoordinationRunStatus::Active);
    }
