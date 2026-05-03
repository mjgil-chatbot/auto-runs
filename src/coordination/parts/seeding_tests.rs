mod seeding_routing_tests {
    use super::*;

    fn make_test_run() -> CoordinationRun {
        CoordinationRun {
            run_id: "test".to_string(),
            root_chat_id: "test".to_string(),
            workspace_path: "/tmp".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "claude".to_string(),
            goal: "test".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 4,
            allow_write_tasks: true,
            require_review_before_apply: true,
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
        }
    }

    #[test]
    fn design_consensus_seeds_no_plan_or_apply_tasks() {
        let run = CoordinationRun {
            run_id: "test-dc".to_string(),
            root_chat_id: "chat".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            goal: "test goal".to_string(),
            workspace_path: "/tmp/test".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            design_doc_path: "docs/design.md".to_string(),
            provider: "claude".to_string(),
            allow_write_tasks: true,
            max_agents: 4,
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
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
        let tasks = seed_initial_tasks(&run);
        let kinds: Vec<_> = tasks.iter().map(|t| &t.kind).collect();
        // Must have survey, draft, converge, materialize — but NO plan/review/apply
        assert!(kinds.contains(&&CoordinationTaskKind::SurveyWorkspace));
        assert!(kinds.contains(&&CoordinationTaskKind::ConvergeDesignDoc));
        assert!(kinds.contains(&&CoordinationTaskKind::MaterializeDesignDoc));
        assert!(
            !kinds.contains(&&CoordinationTaskKind::PlanFlatTasks),
            "DesignConsensus must NOT seed PlanFlatTasks — it stops at the doc"
        );
        assert!(
            !kinds.contains(&&CoordinationTaskKind::ReviewArtifacts),
            "DesignConsensus must NOT seed ReviewArtifacts"
        );
        assert!(
            !kinds.contains(&&CoordinationTaskKind::ApplyApprovedEdits),
            "DesignConsensus must NOT seed ApplyApprovedEdits"
        );
    }

    #[test]
    fn provider_draft_design_doc_path_uses_model_slug_and_design_doc_name() {
        let run = make_test_run();
        assert_eq!(
            provider_draft_design_doc_path(&run, "codex"),
            "docs/_gpt-5.4_design.md"
        );
        assert_eq!(
            provider_draft_design_doc_path(&run, "gemini"),
            "docs/_gemini-3-flash-preview_design.md"
        );
        assert_eq!(
            provider_draft_design_doc_path(&run, "qwen"),
            "docs/_qwen-3.6-free_design.md"
        );
    }

    #[test]
    fn design_consensus_does_not_seed_workspace_copy_task_per_provider_draft() {
        let run = make_test_run();
        let tasks = seed_design_consensus_tasks(&run);
        let draft_materializations = tasks
            .iter()
            .filter(|task| {
                task.kind == CoordinationTaskKind::MaterializeDesignDoc
                    && task.title.ends_with("draft design doc to the workspace")
            })
            .collect::<Vec<_>>();

        assert_eq!(
            draft_materializations.len(),
            0,
            "provider draft docs should now be copied from artifacts by Rust, not via provider tasks"
        );
    }

    #[test]
    fn design_consensus_has_codex_draft_and_codex_canonical_write() {
        let run = make_test_run();
        let tasks = seed_design_consensus_tasks(&run);

        let codex_draft = tasks.iter().find(|task| {
            task.kind == CoordinationTaskKind::DraftDesignDoc
                && task.assigned_provider.as_deref() == Some("codex")
        });
        assert!(
            codex_draft.is_some(),
            "design consensus should include a Codex draft task"
        );

        let canonical_materialize = tasks.iter().find(|task| {
            task.kind == CoordinationTaskKind::MaterializeDesignDoc
                && task.title == "Write canonical design doc to the workspace"
        });
        assert!(
            canonical_materialize.is_some(),
            "design consensus should include a canonical design doc write task"
        );
        assert_eq!(
            canonical_materialize.and_then(|task| task.assigned_provider.as_deref()),
            Some("codex"),
            "the canonical converged write must stay pinned to Codex"
        );
    }

    #[test]
    fn design_consensus_reuses_existing_intermediate_provider_doc() {
        let workspace = tempfile::TempDir::new().unwrap();
        let docs_dir = workspace.path().join("docs");
        fs::create_dir_all(&docs_dir).unwrap();

        let run = CoordinationRun {
            run_id: format!("reuse-{}", Uuid::new_v4()),
            root_chat_id: "chat".to_string(),
            workspace_path: workspace.path().to_string_lossy().into_owned(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "claude".to_string(),
            goal: "test".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 4,
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
        let codex_doc = workspace
            .path()
            .join(provider_draft_design_doc_path(&run, "codex"));
        fs::write(&codex_doc, "# Existing Codex Draft\n").unwrap();

        let tasks = seed_design_consensus_tasks(&run);
        let codex_draft = tasks
            .iter()
            .find(|task| {
                task.kind == CoordinationTaskKind::DraftDesignDoc
                    && task.assigned_provider.as_deref() == Some("codex")
            })
            .expect("codex draft task should exist");
        assert_eq!(codex_draft.status, CoordinationTaskStatus::Completed);
        assert!(
            codex_draft
                .artifact_summary
                .as_deref()
                .unwrap_or_default()
                .contains("Reused existing provider draft design doc"),
            "codex draft should be reused from the existing workspace doc"
        );

        assert!(
            !tasks.iter().any(|task| {
                task.kind == CoordinationTaskKind::MaterializeDesignDoc
                    && task.assigned_provider.as_deref() == Some("codex")
                    && task.title.ends_with("draft design doc to the workspace")
            }),
            "provider drafts should not get a redundant provider-side workspace copy task"
        );

        let notes_path = artifact_notes_path(&run.run_id, &codex_draft.task_id);
        assert!(
            notes_path.is_file(),
            "reused draft should seed dependency notes"
        );
        let notes = fs::read_to_string(notes_path).unwrap();
        assert!(notes.contains("Existing Codex Draft"));

        let _ = fs::remove_dir_all(run_dir(&run.run_id));
    }

    #[test]
    fn design_consensus_provider_draft_titles_are_not_canonical() {
        let run = make_test_run();
        let tasks = seed_design_consensus_tasks(&run);

        let draft_titles = tasks
            .iter()
            .filter(|task| task.kind == CoordinationTaskKind::DraftDesignDoc)
            .map(|task| task.title.as_str())
            .collect::<Vec<_>>();

        assert!(
            !draft_titles.is_empty(),
            "design consensus should seed draft tasks"
        );
        assert!(draft_titles
            .iter()
            .all(|title| title.starts_with("Draft provider design proposal with ")));
        assert!(draft_titles
            .iter()
            .all(|title| !title.contains("canonical")));
    }

    #[test]
    fn design_consensus_runsc_prompts_use_container_visible_workspace_paths() {
        let mut run = make_test_run();
        run.execution_environment = AutonomousExecutionEnvironment::Runsc;
        run.workspace_path = "/tmp/project-almanac".to_string();
        run.goal = format!(
            "create five prompts based on the instructions in {}/instructions and the plan in {}/plan.txt",
            run.workspace_path, run.workspace_path
        );

        let tasks = seed_design_consensus_tasks(&run);
        let draft = tasks
            .iter()
            .find(|task| task.kind == CoordinationTaskKind::DraftDesignDoc)
            .expect("draft task should exist");
        let converge = tasks
            .iter()
            .find(|task| task.kind == CoordinationTaskKind::ConvergeDesignDoc)
            .expect("converge task should exist");

        assert!(draft.prompt.contains("Inspect the workspace at ."));
        assert!(draft.prompt.contains("./instructions"));
        assert!(draft.prompt.contains("./plan.txt"));
        assert!(!draft.prompt.contains(&run.workspace_path));
        assert!(converge.prompt.contains("./instructions"));
        assert!(converge.prompt.contains("./plan.txt"));
        assert!(!converge.prompt.contains(&run.workspace_path));
    }

    #[test]
    fn design_implementation_seeds_plan_and_apply_tasks() {
        let run = CoordinationRun {
            run_id: "test-di".to_string(),
            root_chat_id: "chat".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignImplementation,
            goal: "implement design".to_string(),
            workspace_path: "/tmp/test".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            design_doc_path: "docs/design.md".to_string(),
            provider: "claude".to_string(),
            allow_write_tasks: true,
            max_agents: 4,
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
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
        let tasks = seed_initial_tasks(&run);
        let kinds: Vec<_> = tasks.iter().map(|t| &t.kind).collect();
        // Must have survey, plan, review, apply — but NO draft/converge/materialize
        assert!(kinds.contains(&&CoordinationTaskKind::SurveyWorkspace));
        assert!(
            kinds.contains(&&CoordinationTaskKind::PlanFlatTasks),
            "DesignImplementation must seed PlanFlatTasks"
        );
        assert!(
            kinds.contains(&&CoordinationTaskKind::ReviewArtifacts),
            "DesignImplementation must seed ReviewArtifacts"
        );
        assert!(
            kinds.contains(&&CoordinationTaskKind::ApplyApprovedEdits),
            "DesignImplementation must seed ApplyApprovedEdits"
        );
        assert!(
            !kinds.contains(&&CoordinationTaskKind::DraftDesignDoc),
            "DesignImplementation must NOT seed DraftDesignDoc — it uses an existing doc"
        );
        assert!(
            !kinds.contains(&&CoordinationTaskKind::ConvergeDesignDoc),
            "DesignImplementation must NOT seed ConvergeDesignDoc"
        );
    }

    #[test]
    fn design_consensus_provider_names_uses_registry() {
        let providers = design_consensus_provider_names();
        assert!(!providers.is_empty(), "must resolve at least one provider");
        // All returned providers should support Design in the registry
        let registry = load_capability_registry();
        for pid in &providers {
            let profile = registry.providers.iter().find(|p| &p.provider_id == pid);
            assert!(
                profile.is_some(),
                "provider {} should be in the registry",
                pid
            );
            assert!(
                profile
                    .unwrap()
                    .supported_categories
                    .contains(&TaskCategory::Design),
                "provider {} should support Design category",
                pid
            );
        }
    }

    #[test]
    fn all_provider_names_uses_every_provider_in_registry_order() {
        let providers = all_provider_names();
        let registry = load_capability_registry();
        let expected: Vec<String> = registry
            .providers
            .iter()
            .map(|profile| profile.provider_id.clone())
            .collect();

        assert_eq!(providers, expected);
    }

    #[test]
    fn generated_task_provider_round_robins_all_providers_for_non_ralph_runs() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::DesignImplementation;
        run.provider = "codex".to_string();

        let providers = all_provider_names();
        assert!(providers.len() >= 2, "test requires multiple providers");

        for index in 0..(providers.len() * 2 + 1) {
            assert_eq!(
                resolve_generated_task_provider(&run, Some("claude"), index),
                Some(providers[index % providers.len()].clone())
            );
        }
    }

    #[test]
    fn generated_task_provider_keeps_ralph_provider_selection() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::RalphLoop;
        run.provider = "gemini".to_string();

        assert_eq!(
            resolve_generated_task_provider(&run, Some("claude"), 0),
            Some("claude".to_string())
        );
        assert_eq!(
            resolve_generated_task_provider(&run, None, 0),
            Some("gemini".to_string())
        );
    }

    #[test]
    fn generated_task_provider_ignores_single_provider_override_outside_ralph() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::SingleProvider;
        run.provider = "gemini".to_string();

        assert_eq!(
            resolve_generated_task_provider(&run, Some("claude"), 0),
            Some("gemini".to_string())
        );
    }

    #[test]
    fn claim_with_category_prefers_category_capable_fallback() {
        let mut run = make_test_run();
        run.provider_cooldowns.insert(
            "claude".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "rate limit".to_string(),
            },
        );
        let now = Utc::now();
        let result = resolve_provider_for_task_claim_with_category(
            &run,
            Some("claude"),
            "claude",
            Some(&TaskCategory::Design),
            now,
        );
        assert!(result.is_some());
        assert_ne!(result.as_deref(), Some("claude"));
        // The fallback should support Design
        let registry = load_capability_registry();
        let fb = result.unwrap();
        let profile = registry.providers.iter().find(|p| p.provider_id == fb);
        assert!(
            profile
                .map(|p| p.supported_categories.contains(&TaskCategory::Design))
                .unwrap_or(false),
            "fallback provider should support Design category"
        );
    }

    #[test]
    fn prompt_family_and_task_category_serde_round_trip() {
        let prompt_family = PromptFamily::Mechanics;
        let prompt_family_json = serde_json::to_string(&prompt_family).unwrap();
        assert_eq!(prompt_family_json, "\"mechanics\"");
        let decoded_prompt_family: PromptFamily =
            serde_json::from_str(&prompt_family_json).unwrap();
        assert_eq!(decoded_prompt_family, PromptFamily::Mechanics);

        let task_category = TaskCategory::QuickWrite;
        let task_category_json = serde_json::to_string(&task_category).unwrap();
        assert_eq!(task_category_json, "\"quick_write\"");
        let decoded_task_category: TaskCategory =
            serde_json::from_str(&task_category_json).unwrap();
        assert_eq!(decoded_task_category, TaskCategory::QuickWrite);
    }

    #[test]
    fn ralph_seeded_template_paths_match_parser_contract() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::RalphLoop;
        run.allow_write_tasks = true;
        let execute_template = ralph_execute_template_path(&run.run_id);
        let verify_template = ralph_verify_template_path(&run.run_id);

        let tasks = seed_ralph_loop_tasks(&run);

        let execute_task = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::RalphExecute)
            .expect("must have execute task");
        let verify_task = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::RalphVerify)
            .expect("must have verify task");

        assert!(
            execute_task.write_scope.contains(&execute_template),
            "execute write_scope must include the run-scoped template path"
        );
        assert!(
            verify_task.read_scope.contains(&verify_template),
            "verify read_scope must include the run-scoped template path"
        );
        assert!(
            verify_task.write_scope.contains(&verify_template),
            "verify write_scope must include the run-scoped template path"
        );

        assert_eq!(
            execute_task.write_scope,
            ralph_execute_write_scope(&run.run_id, true)
        );
        assert_eq!(verify_task.read_scope, ralph_verify_read_scope(&run.run_id));
        assert_eq!(
            verify_task.write_scope,
            ralph_verify_write_scope(&run.run_id)
        );
    }

    #[test]
    fn ralph_seeded_task_order_and_dependencies() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::RalphLoop;

        let tasks = seed_ralph_loop_tasks(&run);
        assert_eq!(tasks.len(), 4, "survey + plan + execute + verify");

        let survey = &tasks[0];
        let plan = &tasks[1];
        let execute = &tasks[2];
        let verify = &tasks[3];

        assert_eq!(survey.kind, CoordinationTaskKind::SurveyWorkspace);
        assert_eq!(plan.kind, CoordinationTaskKind::RalphPlan);
        assert_eq!(execute.kind, CoordinationTaskKind::RalphExecute);
        assert_eq!(verify.kind, CoordinationTaskKind::RalphVerify);

        assert_eq!(survey.status, CoordinationTaskStatus::Queued);
        assert_eq!(plan.status, CoordinationTaskStatus::Blocked);
        assert_eq!(execute.status, CoordinationTaskStatus::Blocked);
        assert_eq!(verify.status, CoordinationTaskStatus::Blocked);

        assert!(plan.dependencies.contains(&survey.task_id));
        assert!(execute.dependencies.contains(&plan.task_id));
        assert!(verify.dependencies.contains(&plan.task_id));
        assert!(verify.dependencies.contains(&execute.task_id));
    }

    #[test]
    fn ralph_allow_write_tasks_true_includes_edit_and_dot_scope() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::RalphLoop;
        run.allow_write_tasks = true;

        let tasks = seed_ralph_loop_tasks(&run);
        let execute = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::RalphExecute)
            .unwrap();

        assert!(
            execute.allowed_tools.contains(&"edit".to_string()),
            "allow_write_tasks=true must grant edit tool"
        );
        assert!(
            execute.write_scope.contains(&".".to_string()),
            "allow_write_tasks=true must grant workspace write scope"
        );
    }

    #[test]
    fn ralph_allow_write_tasks_false_excludes_edit_and_dot_scope() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::RalphLoop;
        run.allow_write_tasks = false;

        let tasks = seed_ralph_loop_tasks(&run);
        let execute = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::RalphExecute)
            .unwrap();

        assert!(
            !execute.allowed_tools.contains(&"edit".to_string()),
            "allow_write_tasks=false must not grant edit tool"
        );
        assert!(
            !execute.write_scope.contains(&".".to_string()),
            "allow_write_tasks=false must not grant workspace write scope"
        );
        assert!(
            execute
                .write_scope
                .contains(&ralph_execute_template_path(&run.run_id)),
            "template path must still be writable even without allow_write_tasks"
        );
    }

    #[test]
    fn perf_hill_climber_seeds_baseline_and_parse_tasks() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;
        run.workspace_path = "/tmp/test-workspace".to_string();
        run.design_doc_path = "docs/perf-ideas.md".to_string();

        let tasks = seed_perf_hill_climber_tasks(&run);
        assert_eq!(tasks.len(), 2, "baseline + parse");

        let baseline = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
            .expect("PerfBaseline task missing");
        let parse = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfParse)
            .expect("PerfParse task missing");

        assert_eq!(baseline.status, CoordinationTaskStatus::Queued);
        assert_eq!(parse.status, CoordinationTaskStatus::Blocked);

        assert!(
            parse.dependencies.contains(&baseline.task_id),
            "PerfParse must depend on PerfBaseline"
        );

        assert_eq!(baseline.iteration, Some(0));

        assert!(
            baseline.prompt.contains("/tmp/test-workspace"),
            "baseline prompt must reference workspace"
        );
        assert!(
            baseline.prompt.contains("/tmp/perf_baseline.json"),
            "baseline prompt must reference output path"
        );

        assert!(
            parse.prompt.contains("docs/perf-ideas.md"),
            "parse prompt must reference design doc path"
        );
    }

    #[test]
    fn perf_hill_climber_seeding_is_wired_in_seed_initial_tasks() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;

        let tasks = seed_initial_tasks(&run);
        assert_eq!(
            tasks.len(),
            2,
            "seed_initial_tasks must delegate to seed_perf_hill_climber_tasks"
        );
        assert!(tasks
            .iter()
            .any(|t| t.kind == CoordinationTaskKind::PerfBaseline));
        assert!(tasks
            .iter()
            .any(|t| t.kind == CoordinationTaskKind::PerfParse));
    }

    #[test]
    fn perf_hill_climber_bench_scan_path_defaults_to_snap() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;
        run.notes = String::new();

        let tasks = seed_perf_hill_climber_tasks(&run);
        let baseline = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
            .unwrap();
        assert!(
            baseline.prompt.contains("/snap"),
            "default scan path must be /snap; got: {}",
            baseline.prompt
        );
    }

    #[test]
    fn perf_hill_climber_bench_scan_path_overrides_from_notes() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;
        run.notes = "bench_scan_path=/mnt/data\nother=stuff".to_string();

        let tasks = seed_perf_hill_climber_tasks(&run);
        let baseline = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
            .unwrap();
        assert!(
            baseline.prompt.contains("/mnt/data"),
            "notes bench_scan_path must override default; got: {}",
            baseline.prompt
        );
        assert!(
            !baseline.prompt.contains("/snap"),
            "default scan path must not appear when overridden"
        );
    }

    #[test]
    fn perf_hill_climber_uses_explicit_bench_script_path_from_goal() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;
        run.goal = "iterate on performance, use /home/developer/git/ff-search/bench.sh".to_string();

        let tasks = seed_perf_hill_climber_tasks(&run);
        let baseline = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
            .unwrap();
        assert!(
            baseline
                .prompt
                .contains("'/home/developer/git/ff-search/bench.sh'"),
            "explicit bench script path from goal must be used; got: {}",
            baseline.prompt
        );
    }

    #[test]
    fn perf_hill_climber_prefers_bench_script_path_from_notes() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;
        run.goal = "iterate on performance, use /home/developer/git/ff-search/bench.sh".to_string();
        run.notes = "bench_script_path=/tmp/custom bench.sh".to_string();

        let tasks = seed_perf_hill_climber_tasks(&run);
        let baseline = tasks
            .iter()
            .find(|t| t.kind == CoordinationTaskKind::PerfBaseline)
            .unwrap();
        assert!(
            baseline.prompt.contains("'/tmp/custom bench.sh'"),
            "notes bench_script_path must override goal fallback; got: {}",
            baseline.prompt
        );
        assert!(
            !baseline
                .prompt
                .contains("'/home/developer/git/ff-search/bench.sh'"),
            "goal fallback path must not appear when bench_script_path note is set"
        );
    }

    #[test]
    fn perf_hill_climber_reports_record_temp_outputs_as_changed_files() {
        let baseline =
            perf_baseline_report(r#"{"parallel_median_s":1.2}"#).expect("baseline should parse");
        assert_eq!(
            baseline.changed_files,
            vec!["/tmp/perf_baseline.json".to_string()]
        );

        let bench = perf_bench_report(r#"{"idea_index":3,"parallel_median_s":0.9}"#)
            .expect("bench should parse");
        assert_eq!(
            bench.changed_files,
            vec!["/tmp/perf_bench_3.json".to_string()]
        );
        assert_eq!(
            bench.summary,
            "Bench idea 4: primary median 0.90s (unknown)"
        );

        let evaluate = perf_evaluate_report(
            r#"{"keep":true,"idea_index":4,"reason":"faster","improvement_pct":5.0}"#,
        )
        .expect("PerfEvaluate JSON should parse");
        assert_eq!(
            evaluate.changed_files,
            vec!["/tmp/perf_evaluate_4.json".to_string()]
        );
    }

    #[test]
    fn perf_hill_climber_normalizes_legacy_benchmark_output() {
        let artifact = parse_perf_benchmark_artifact(
            r#"{
                "parallel_median_s": 1.2,
                "parallel_min_s": 1.0,
                "parallel_sub_trees_median_s": 1.1,
                "parallel_sub_trees_min_s": 0.9,
                "raw": "parallel: min=1.0 median=1.2"
            }"#,
        )
        .expect("legacy benchmark output should normalize");

        assert_eq!(artifact.schema_version, PERF_BENCHMARK_SCHEMA_VERSION);
        assert_eq!(artifact.correctness.status, PerfCorrectnessStatus::Unknown);
        assert_eq!(artifact.metrics.primary_case, PERF_PRIMARY_BENCH_CASE);
        assert_eq!(artifact.primary_case_median_s(), Some(1.2));
        assert_eq!(artifact.metrics.cases.len(), 2);
    }

    #[test]
    fn perf_hill_climber_baseline_prompt_requests_standard_schema() {
        let mut run = make_test_run();
        run.workflow_mode = AutonomousWorkflowMode::PerfHillClimber;

        let baseline = seed_perf_hill_climber_tasks(&run)
            .into_iter()
            .find(|task| task.kind == CoordinationTaskKind::PerfBaseline)
            .expect("perf baseline task");

        assert!(baseline.prompt.contains("\"schema_version\": 1"));
        assert!(baseline.prompt.contains("\"correctness\""));
        assert!(baseline.prompt.contains("\"metrics\""));
    }
}
