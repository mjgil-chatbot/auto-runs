mod lifecycle_routing_tests {
    use super::*;
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    struct LocalTempDir {
        path: PathBuf,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match self.previous.as_ref() {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    impl LocalTempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("auto-runs-test-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for LocalTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn make_test_run_for_lifecycle() -> CoordinationRun {
        CoordinationRun {
            run_id: "lc-test".to_string(),
            root_chat_id: "lc-test".to_string(),
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

    fn init_git_workspace() -> LocalTempDir {
        let workspace = LocalTempDir::new();
        let rust_debug_root = workspace.path().parent().unwrap().join("rust-debug");
        fs::create_dir_all(rust_debug_root.join("src")).unwrap();
        fs::write(
            rust_debug_root.join("Cargo.toml"),
            "[package]\nname = \"rust-debug\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\ntracing-integration = []\n",
        )
        .unwrap();
        fs::write(rust_debug_root.join("src/lib.rs"), "pub fn init() {}\n").unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nrust-debug = { path = \"../rust-debug\", features = [\"tracing-integration\"] }\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("bench.sh"),
            "#!/usr/bin/env bash\nexit 0\n",
        )
        .unwrap();
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(
            workspace.path().join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .unwrap();
        run_git(workspace.path(), &["init"]);
        run_git(
            workspace.path(),
            &["config", "user.email", "tests@example.com"],
        );
        run_git(
            workspace.path(),
            &["config", "user.name", "Coordination Tests"],
        );
        run_git(workspace.path(), &["add", "."]);
        run_git(workspace.path(), &["commit", "-m", "initial"]);
        workspace
    }

    fn run_git(workspace: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(workspace)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    #[test]
    fn resolve_claim_without_category_returns_requested() {
        let run = make_test_run_for_lifecycle();
        let now = Utc::now();
        let result = resolve_provider_for_task_claim(&run, Some("codex"), "claude", now);
        assert_eq!(result.as_deref(), Some("codex"));
    }

    #[test]
    fn resolve_claim_falls_back_on_cooldown() {
        let mut run = make_test_run_for_lifecycle();
        run.provider_cooldowns.insert(
            "claude".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "rate limit".to_string(),
            },
        );
        let now = Utc::now();
        let result = resolve_provider_for_task_claim(&run, Some("claude"), "claude", now);
        assert!(result.is_some());
        assert_ne!(result.as_deref(), Some("claude"));
    }

    #[test]
    fn resolve_claim_with_category_uses_category_fallback() {
        let mut run = make_test_run_for_lifecycle();
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
            Some(&TaskCategory::Verify),
            now,
        );
        assert!(result.is_some());
        let fb = result.unwrap();
        assert_eq!(fb, "codex");
        // Verify the fallback supports Verify
        let registry = load_capability_registry();
        if let Some(profile) = registry.providers.iter().find(|p| p.provider_id == fb) {
            assert!(
                profile.supported_categories.contains(&TaskCategory::Verify),
                "category-aware fallback should support Verify"
            );
        }
    }

    #[test]
    fn cooldown_pruning_removes_expired_entries() {
        let mut cooldowns = BTreeMap::new();
        cooldowns.insert(
            "expired".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() - chrono::Duration::hours(1),
                reason: "old".to_string(),
            },
        );
        cooldowns.insert(
            "active".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "new".to_string(),
            },
        );
        let changed = prune_expired_provider_cooldown_map(&mut cooldowns, Utc::now());
        assert!(changed);
        assert!(!cooldowns.contains_key("expired"));
        assert!(cooldowns.contains_key("active"));
    }

    #[test]
    fn category_fallback_skips_non_supporting_providers() {
        let mut run = make_test_run_for_lifecycle();
        // Cool down all providers except minimax
        for pid in &["claude", "codex", "gemini", "qwen"] {
            run.provider_cooldowns.insert(
                pid.to_string(),
                ProviderCooldown {
                    unavailable_until: Utc::now() + chrono::Duration::hours(1),
                    reason: "rate limit".to_string(),
                },
            );
        }
        let now = Utc::now();
        // Design category: minimax doesn't have Design as preferred in providers.json
        // but may support it — the fallback should return it if it's the only available
        let result =
            select_category_fallback_provider(&run, "claude", &TaskCategory::QuickWrite, now);
        // minimax supports QuickWrite
        assert_eq!(result.as_deref(), Some("minimax"));
    }

    #[test]
    fn ensure_workspace_provider_capability_registry_refreshes_stale_workspace_copy() {
        let workspace = LocalTempDir::new();
        let mut run = make_test_run_for_lifecycle();
        run.workspace_path = workspace.path().display().to_string();

        let registry_path = workspace.path().join("workflow-registry/providers.json");
        fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        fs::write(
            &registry_path,
            r#"{"providers":[{"provider_id":"claude","model_override":"old","prompt_family":"principles","supported_categories":["design"],"preferred_categories":["design"],"fallback_chain":["codex"],"cooldown_eligible":true,"max_concurrency":1,"cost_tier":"premium","notes":"stale"}]}"#,
        )
        .unwrap();

        ensure_workspace_provider_capability_registry(&run).unwrap();

        let refreshed = fs::read_to_string(&registry_path).unwrap();
        assert!(refreshed.contains("\"provider_id\": \"nemotron\""));
    }

    #[test]
    fn resolve_claim_with_category_prefers_preferred_available_provider_when_hint_is_unsupported() {
        let run = make_test_run_for_lifecycle();
        let now = Utc::now();
        // minimax does NOT support DeepWrite, so the resolver should fall back
        // to the first preferred+available provider for DeepWrite — which is claude.
        let result = resolve_provider_for_task_claim_with_category(
            &run,
            Some("minimax"),
            "claude",
            Some(&TaskCategory::DeepWrite),
            now,
        );
        assert_eq!(result.as_deref(), Some("claude"));
    }

    #[test]
    fn validate_run_input_allows_perf_hill_climber() {
        validate_run_input(&UpsertRunInput {
            existing_run_id: None,
            root_chat_id: "chat".to_string(),
            workspace_path: ".".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "improve performance".to_string(),
            workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
        })
        .expect("perf hill climber should be accepted once coordination wiring is live");
    }

    #[test]
    fn upsert_run_creates_perf_worktree_and_perf_tasks_use_it() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());
        let workspace = init_git_workspace();

        let snapshot = upsert_run(UpsertRunInput {
            existing_run_id: None,
            root_chat_id: "chat".to_string(),
            workspace_path: workspace.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "improve performance".to_string(),
            workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
        })
        .expect("perf run should be created");

        let worktree_root =
            perf_worktree_root_for_run(&snapshot.run).expect("perf worktree root note");
        let task_workspace = perf_workspace_path_for_run(&snapshot.run);
        assert!(Path::new(&worktree_root).is_dir());
        assert!(Path::new(&task_workspace).is_dir());
        assert_ne!(task_workspace, snapshot.run.workspace_path);
        assert!(snapshot.tasks.iter().all(|task| task.cwd == task_workspace));
        let baseline = snapshot
            .tasks
            .iter()
            .find(|task| task.kind == CoordinationTaskKind::PerfBaseline)
            .expect("baseline task");
        assert!(baseline.prompt.contains(&task_workspace));
        assert!(!baseline.prompt.contains(&snapshot.run.workspace_path));
        let worktree_manifest = fs::read_to_string(Path::new(&task_workspace).join("Cargo.toml"))
            .expect("worktree Cargo.toml");
        let sibling_dep = workspace.path().parent().unwrap().join("rust-debug");
        let sibling_dep_text = sibling_dep.to_string_lossy().into_owned();
        assert!(
            worktree_manifest.contains(&sibling_dep_text),
            "worktree manifest should rewrite sibling path dependencies to absolute paths"
        );
    }

    #[test]
    fn stop_run_removes_perf_worktree() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());
        let workspace = init_git_workspace();

        let snapshot = upsert_run(UpsertRunInput {
            existing_run_id: None,
            root_chat_id: "chat".to_string(),
            workspace_path: workspace.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "improve performance".to_string(),
            workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
        })
        .expect("perf run should be created");

        let worktree_root =
            perf_worktree_root_for_run(&snapshot.run).expect("perf worktree root note");
        assert!(Path::new(&worktree_root).exists());
        stop_run(&snapshot.run.run_id, "test stop").expect("stop run");
        assert!(!Path::new(&worktree_root).exists());
    }

    #[test]
    fn restart_failed_perf_tasks_recreates_deleted_worktree() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());
        let workspace = init_git_workspace();

        let snapshot = upsert_run(UpsertRunInput {
            existing_run_id: None,
            root_chat_id: "chat".to_string(),
            workspace_path: workspace.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "improve performance".to_string(),
            workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
        })
        .expect("perf run should be created");

        let worktree_root =
            perf_worktree_root_for_run(&snapshot.run).expect("perf worktree root note");
        let task_workspace = perf_workspace_path_for_run(&snapshot.run);
        let mut baseline = snapshot
            .tasks
            .iter()
            .find(|task| task.kind == CoordinationTaskKind::PerfBaseline)
            .cloned()
            .expect("baseline task");
        baseline.status = CoordinationTaskStatus::Failed;
        baseline.artifact_summary = Some("provider failed".to_string());
        baseline.updated_at = Utc::now();
        write_json_atomic(
            &task_path(&snapshot.run.run_id, &baseline.task_id),
            &baseline,
        )
        .expect("persist failed baseline");

        stop_run(&snapshot.run.run_id, "test stop").expect("stop run");
        assert!(!Path::new(&worktree_root).exists());

        let restarted =
            restart_failed_tasks(&snapshot.run.run_id).expect("restart should recreate worktree");
        let restarted_baseline = restarted
            .tasks
            .iter()
            .find(|task| task.task_id == baseline.task_id)
            .expect("restarted baseline task");

        assert_eq!(restarted.run.status, CoordinationRunStatus::Active);
        assert_eq!(restarted_baseline.status, CoordinationTaskStatus::Queued);
        assert!(Path::new(&worktree_root).is_dir());
        assert!(Path::new(&task_workspace).is_dir());
        assert_eq!(restarted_baseline.cwd, task_workspace);
    }

    #[test]
    fn ensure_perf_workspace_ready_recreates_invalid_worktree() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());
        let workspace = init_git_workspace();

        let mut snapshot = upsert_run(UpsertRunInput {
            existing_run_id: None,
            root_chat_id: "chat".to_string(),
            workspace_path: workspace.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "improve performance".to_string(),
            workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
        })
        .expect("perf run should be created");

        let worktree_root =
            perf_worktree_root_for_run(&snapshot.run).expect("perf worktree root note");
        let git_file = Path::new(&worktree_root).join(".git");
        fs::remove_file(&git_file).expect("remove linked worktree git file");
        let broken_status = Command::new("git")
            .arg("-C")
            .arg(&worktree_root)
            .args(["rev-parse", "--show-toplevel"])
            .status()
            .expect("git rev-parse should run");
        assert!(
            !broken_status.success(),
            "worktree should be invalid after .git removal"
        );

        let recreated =
            ensure_perf_workspace_ready(&mut snapshot.run).expect("recreate invalid worktree");
        assert!(recreated, "invalid perf worktree should be recreated");
        assert!(
            git_file.exists(),
            "recreated worktree should restore .git link"
        );
        run_git(Path::new(&worktree_root), &["status", "--short"]);
    }

    #[test]
    fn effective_runsc_workspace_includes_uses_only_explicit_entries() {
        let mut run = make_test_run_for_lifecycle();
        run.execution_environment = AutonomousExecutionEnvironment::Runsc;
        run.workflow_mode = AutonomousWorkflowMode::DesignConsensus;
        run.runsc_workspace_includes = vec![
            "instructions".to_string(),
            "plan.txt".to_string(),
            " instructions ".to_string(),
            "".to_string(),
        ];
        let task = CoordinationTask {
            task_id: "task".to_string(),
            run_id: run.run_id.clone(),
            parent_task_id: None,
            kind: CoordinationTaskKind::SurveyWorkspace,
            assigned_provider: Some("codex".to_string()),
            task_category: Some(TaskCategory::Survey),
            provider_hint: None,
            required_role: None,
            iteration: None,
            title: "Survey".to_string(),
            prompt: "Prompt".to_string(),
            cwd: run.workspace_path.clone(),
            allowed_tools: vec!["filesystem".to_string()],
            read_scope: vec!["**".to_string(), "src/**".to_string()],
            write_scope: vec!["docs/design.md".to_string()],
            acceptance_checks: Vec::new(),
            review_metadata: None,
            dependencies: Vec::new(),
            execution_generation: default_execution_generation(),
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: 3,
            publish_attempt_count: 0,
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::SurveyWorkspace,
            )),
            status: CoordinationTaskStatus::Queued,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let includes = effective_runsc_workspace_includes(&run, Some(&task));
        assert_eq!(
            includes,
            vec!["instructions".to_string(), "plan.txt".to_string()]
        );
    }

    #[test]
    fn persist_selected_tasks_does_not_overwrite_unselected_active_task() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());

        let run_id = Uuid::new_v4().to_string();
        fs::create_dir_all(tasks_dir(&run_id)).unwrap();

        let now = Utc::now();
        let blocked_task = CoordinationTask {
            task_id: "blocked-task".to_string(),
            run_id: run_id.clone(),
            title: "Blocked task".to_string(),
            status: CoordinationTaskStatus::Blocked,
            created_at: now,
            updated_at: now,
            ..CoordinationTask::default()
        };
        let active_task = CoordinationTask {
            task_id: "active-task".to_string(),
            run_id: run_id.clone(),
            title: "Active task".to_string(),
            status: CoordinationTaskStatus::Running,
            claimed_by: Some(4242),
            created_at: now,
            updated_at: now,
            ..CoordinationTask::default()
        };

        write_json_atomic(&task_path(&run_id, &blocked_task.task_id), &blocked_task).unwrap();
        write_json_atomic(&task_path(&run_id, &active_task.task_id), &active_task).unwrap();

        let mut stale_snapshot_active = active_task.clone();
        stale_snapshot_active.status = CoordinationTaskStatus::Queued;
        stale_snapshot_active.claimed_by = None;
        stale_snapshot_active.updated_at = now + chrono::Duration::seconds(5);

        let mut updated_blocked = blocked_task.clone();
        updated_blocked.status = CoordinationTaskStatus::Queued;
        updated_blocked.updated_at = now + chrono::Duration::seconds(5);

        persist_selected_tasks(
            &run_id,
            &[updated_blocked.clone(), stale_snapshot_active],
            &[updated_blocked.task_id.clone()],
        )
        .unwrap();

        let persisted_blocked = load_task(&run_id, &updated_blocked.task_id).unwrap();
        assert_eq!(persisted_blocked.status, CoordinationTaskStatus::Queued);

        let persisted_active = load_task(&run_id, &active_task.task_id).unwrap();
        assert_eq!(persisted_active.status, CoordinationTaskStatus::Running);
        assert_eq!(persisted_active.claimed_by, Some(4242));
    }

    #[test]
    fn complete_task_clears_claimed_by_for_completed_task() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());

        let run_id = Uuid::new_v4().to_string();
        let task_id = Uuid::new_v4().to_string();
        let worker_pid = 4242;
        let now = Utc::now();

        fs::create_dir_all(tasks_dir(&run_id)).unwrap();
        fs::create_dir_all(claims_dir(&run_id)).unwrap();
        write_json_atomic(
            &run_path(&run_id),
            &CoordinationRun {
                run_id: run_id.clone(),
                root_chat_id: "chat".to_string(),
                workspace_path: home.path().display().to_string(),
                execution_environment: AutonomousExecutionEnvironment::Host,
                runsc_workspace_includes: Vec::new(),
                provider: "codex".to_string(),
                goal: "test".to_string(),
                workflow_mode: AutonomousWorkflowMode::DesignConsensus,
                design_doc_path: "docs/design.md".to_string(),
                completion_criteria: String::new(),
                max_iterations: 1,
                status: CoordinationRunStatus::Active,
                created_at: now,
                updated_at: now,
                created_by_instance: None,
                max_agents: 1,
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
                task_ids: vec![task_id.clone()],
            processed_mutation_batch_ids: BTreeSet::new(),
            },
        )
        .unwrap();
        write_json_atomic(
            &task_path(&run_id, &task_id),
            &CoordinationTask {
                task_id: task_id.clone(),
                run_id: run_id.clone(),
                kind: CoordinationTaskKind::GeneratedWriteTask,
                assigned_provider: Some("codex".to_string()),
                task_category: Some(TaskCategory::DeepWrite),
                title: "Write slice".to_string(),
                prompt: "write".to_string(),
                cwd: home.path().display().to_string(),
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                read_scope: vec!["src/lib.rs".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                status: CoordinationTaskStatus::Running,
                attempt_count: 1,
                max_attempts: 3,
                claimed_by: Some(worker_pid),
                created_at: now,
                updated_at: now,
                ..CoordinationTask::default()
            },
        )
        .unwrap();
        write_json_atomic(
            &claim_path(&run_id, &task_id),
            &TaskClaim {
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                provider: "codex".to_string(),
                instance_pid: worker_pid,
                launcher_pid: Some(worker_pid),
                lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
                worker_pid: Some(worker_pid),
                claimed_at: now,
                worker_spawned_at: Some(now),
                worker_adopted_at: Some(now),
                execution_started_at: Some(now),
                worker_exited_at: None,
                worker_exit_detail: None,
                last_heartbeat_at: now,
            },
        )
        .unwrap();

        complete_task(
            &run_id,
            &task_id,
            worker_pid,
            TaskArtifact {
                run_id: run_id.clone(),
                task_id: task_id.clone(),
                provider: None,
                task_category: None,
                fallback_from: None,
                summary: "surveyed workspace".to_string(),
                body: "workspace looks good".to_string(),
                completed_at: now,
                report: TaskExecutionReport {
                    outcome: CoordinationTaskOutcome::CompletedNoChange,
                    summary: "surveyed workspace".to_string(),
                    details: "workspace looks good".to_string(),
                    changed_files: vec![],
                    ..Default::default()
                },
            },
        )
        .unwrap();

        let completed_task = load_task(&run_id, &task_id).unwrap();
        assert_eq!(completed_task.status, CoordinationTaskStatus::Completed);
        assert_eq!(completed_task.claimed_by, None);
        assert!(!claim_path(&run_id, &task_id).exists());
    }

    #[test]
    fn builtin_task_failure_cleans_up_claim_without_waiting_for_stale_recovery() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let home = LocalTempDir::new();
        let _home_guard = EnvVarGuard::set("HOME", home.path());

        let run_id = Uuid::new_v4().to_string();
        fs::create_dir_all(tasks_dir(&run_id)).unwrap();
        fs::create_dir_all(claims_dir(&run_id)).unwrap();

        let now = Utc::now();
        let run = CoordinationRun {
            run_id: run_id.clone(),
            root_chat_id: "chat".to_string(),
            workspace_path: home.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "test".to_string(),
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
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
            provider_cooldowns: BTreeMap::new(),
            task_ids: vec!["survey-task".to_string()],
        processed_mutation_batch_ids: BTreeSet::new(),
        };
        let cwd_file = home.path().join("not-a-directory");
        fs::write(&cwd_file, "blocking file").unwrap();

        let task = CoordinationTask {
            task_id: "survey-task".to_string(),
            run_id: run_id.clone(),
            kind: CoordinationTaskKind::SurveyWorkspace,
            assigned_provider: Some("codex".to_string()),
            task_category: Some(TaskCategory::Survey),
            title: "Survey workspace".to_string(),
            prompt: "survey".to_string(),
            cwd: cwd_file.to_string_lossy().into_owned(),
            allowed_tools: vec!["filesystem".to_string()],
            read_scope: vec!["**".to_string()],
            write_scope: Vec::new(),
            execution_generation: default_execution_generation(),
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::SurveyWorkspace,
            )),
            status: CoordinationTaskStatus::Queued,
            created_at: now,
            updated_at: now,
            ..CoordinationTask::default()
        };
        write_json_atomic(&run_path(&run_id), &run).unwrap();
        write_json_atomic(&task_path(&run_id, &task.task_id), &task).unwrap();

        let err = drive_run_for_instance(&run_id, 41, "codex").unwrap_err();
        assert!(
            err.to_string().contains("workspace"),
            "unexpected error: {err}"
        );

        let reloaded = load_task(&run_id, &task.task_id).unwrap();
        assert_eq!(reloaded.status, CoordinationTaskStatus::Failed);
        assert_eq!(reloaded.attempt_count, 1);
        assert!(reloaded.artifact_summary.is_some());
        assert!(!claim_path(&run_id, &task.task_id).exists());
    }
}
