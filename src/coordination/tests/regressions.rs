// ═══════════════════════════════════════════════════════════════════
// Regression tests for previously fixed bugs.
//
// Each section cites the commit that introduced the bug fix so the
// test purpose is traceable.
// ═══════════════════════════════════════════════════════════════════

// ── de2feaf: brace expansion for task scope files ────────────────

#[test]
fn expand_braces_single_group() {
    let result = expand_braces("src/{auth,error,model}.rs").unwrap();
    assert_eq!(
        result,
        vec![
            "src/auth.rs".to_string(),
            "src/error.rs".to_string(),
            "src/model.rs".to_string(),
        ]
    );
}

#[test]
fn expand_braces_trims_whitespace_in_alternatives() {
    let result = expand_braces("src/{ a , b }.rs").unwrap();
    assert_eq!(result, vec!["src/a.rs".to_string(), "src/b.rs".to_string()]);
}

#[test]
fn expand_braces_no_braces_returns_none() {
    assert!(expand_braces("src/lib.rs").is_none());
}

#[test]
fn expand_braces_empty_braces() {
    let result = expand_braces("src/{}.rs").unwrap();
    assert_eq!(result, vec!["src/.rs".to_string()]);
}

#[test]
fn scope_allows_brace_expanded_path() {
    // Before de2feaf, brace patterns were not expanded,
    // so "src/{auth,model}.rs" would never match "src/auth.rs".
    assert!(scope_allows_relative_path(
        "src/{auth,model}.rs",
        "src/auth.rs"
    ));
    assert!(scope_allows_relative_path(
        "src/{auth,model}.rs",
        "src/model.rs"
    ));
    assert!(!scope_allows_relative_path(
        "src/{auth,model}.rs",
        "src/config.rs"
    ));
}

#[test]
fn scope_allows_exact_match() {
    assert!(scope_allows_relative_path("src/lib.rs", "src/lib.rs"));
}

#[test]
fn scope_allows_directory_prefix() {
    assert!(scope_allows_relative_path("src", "src/lib.rs"));
}

#[test]
fn scope_dot_allows_anything() {
    assert!(scope_allows_relative_path(".", "src/lib.rs"));
}

#[test]
fn scope_double_star_allows_anything() {
    assert!(scope_allows_relative_path("**", "src/deep/nested.rs"));
}

#[test]
fn scope_wildcard_prefix_matches() {
    assert!(scope_allows_relative_path("src/*", "src/lib.rs"));
}

#[test]
fn scope_rejects_outside_path() {
    assert!(!scope_allows_relative_path("src/auth.rs", "tests/main.rs"));
}

// ── normalize_scope ──────────────────────────────────────────────

#[test]
fn normalize_scope_trims_slashes() {
    assert_eq!(normalize_scope("/src/lib.rs/"), "src/lib.rs");
}

#[test]
fn normalize_scope_empty_becomes_dot() {
    assert_eq!(normalize_scope(""), ".");
    assert_eq!(normalize_scope("  "), ".");
    assert_eq!(normalize_scope("/"), ".");
}

// ── file_is_within_write_scope ─────────────────────────────────────

fn make_task_with_scope(cwd: &str, write_scope: Vec<String>) -> CoordinationTask {
    CoordinationTask {
        task_id: "task-scope-test".to_string(),
        run_id: "run-1".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: None,
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "scope test".to_string(),
        prompt: "test".to_string(),
        cwd: cwd.to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope,
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: DEFAULT_TASK_MAX_ATTEMPTS,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::GeneratedWriteTask,
        )),
        status: CoordinationTaskStatus::Running,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn file_is_within_write_scope_resolves_scope_against_cwd() {
    // Regression test: scope paths like "home/developer/git/repo/Cargo.toml" were not
    // being resolved against task.cwd, so they failed to match changed_files
    // that were simple relative paths like "Cargo.toml".
    let task = make_task_with_scope(
        "/home/developer/git/openrouter-cli",
        vec![
            "home/developer/git/openrouter-cli/Cargo.toml".to_string(),
            "home/developer/git/openrouter-cli/src/main.rs".to_string(),
            "home/developer/git/openrouter-cli/src/cli.rs".to_string(),
        ],
    );

    // These changed_files are relative to task.cwd and should now match
    assert!(
        file_is_within_write_scope(&task, "Cargo.toml"),
        "Cargo.toml should be within write_scope after resolving scope against cwd"
    );
    assert!(
        file_is_within_write_scope(&task, "src/main.rs"),
        "src/main.rs should be within write_scope"
    );
    assert!(
        file_is_within_write_scope(&task, "src/cli.rs"),
        "src/cli.rs should be within write_scope"
    );

    // Files not in scope should be rejected
    assert!(
        !file_is_within_write_scope(&task, "src/lib.rs"),
        "src/lib.rs should be outside write_scope"
    );
}

#[test]
fn file_is_within_write_scope_with_already_relative_scope() {
    // When write_scope entries are already relative to cwd (no path prefix),
    // they should still work correctly.
    let task = make_task_with_scope(
        "/home/developer/git/rust-chatbot",
        vec![
            "src/lib.rs".to_string(),
            "src/main.rs".to_string(),
            "Cargo.toml".to_string(),
        ],
    );

    assert!(file_is_within_write_scope(&task, "src/lib.rs"));
    assert!(file_is_within_write_scope(&task, "src/main.rs"));
    assert!(file_is_within_write_scope(&task, "Cargo.toml"));
    assert!(!file_is_within_write_scope(&task, "src/bin/"));
}

#[test]
fn file_is_within_write_scope_directory_scope_matches_files() {
    // A directory in write_scope should allow any file within it.
    let task = make_task_with_scope(
        "/home/developer/git/repo",
        vec!["src".to_string(), "tests".to_string()],
    );

    assert!(file_is_within_write_scope(&task, "src/lib.rs"));
    assert!(file_is_within_write_scope(&task, "src/bin/main.rs"));
    assert!(file_is_within_write_scope(&task, "tests/integration.rs"));
    assert!(!file_is_within_write_scope(&task, "Cargo.toml"));
}

#[test]
fn file_is_within_write_scope_with_dot_scope() {
    // Dot scope should allow anything.
    let task = make_task_with_scope("/home/developer/git/repo", vec![".".to_string()]);

    assert!(file_is_within_write_scope(&task, "Cargo.toml"));
    assert!(file_is_within_write_scope(&task, "src/lib.rs"));
    assert!(file_is_within_write_scope(
        &task,
        "any/deeply/nested/path.rs"
    ));
}

#[test]
fn file_is_within_write_scope_with_double_star() {
    // Double star scope should allow anything.
    let task = make_task_with_scope("/home/developer/git/repo", vec!["**".to_string()]);

    assert!(file_is_within_write_scope(&task, "Cargo.toml"));
    assert!(file_is_within_write_scope(&task, "src/lib.rs"));
    assert!(file_is_within_write_scope(
        &task,
        "any/deeply/nested/path.rs"
    ));
}

// ── b01c45d: extract_task_execution_report_json ──────────────────

#[test]
fn extract_report_json_finds_object_with_required_fields() {
    let body = r#"Some preamble text.
{"outcome":"completed_no_change","summary":"done","details":"nothing changed","changed_files":[],"tests_run":[],"warnings":[],"blockers":[],"remaining_gaps":[],"design_requirements_satisfied":[],"fix_tasks":[]}
Some trailing text."#;
    let json = extract_task_execution_report_json(body).expect("should find valid JSON");
    assert!(json.contains("\"outcome\""));
    assert!(json.contains("\"summary\""));
    assert!(json.contains("\"details\""));
}

#[test]
fn extract_report_json_skips_non_report_json() {
    // Before b01c45d, the first JSON object was always used even if
    // it lacked required fields (outcome, summary, details).
    let body = r#"{"thinking":"some thoughts"} followed by {"outcome":"completed_no_change","summary":"all good","details":"verified"}"#;
    let json = extract_task_execution_report_json(body).expect("should skip first object");
    assert!(json.contains("\"outcome\""));
    assert!(json.contains("\"summary\""));
    assert!(json.contains("\"details\""));
}

#[test]
fn extract_report_json_returns_none_when_no_valid_object() {
    let body = r#"No JSON here, just text."#;
    assert!(extract_task_execution_report_json(body).is_none());
}

#[test]
fn extract_report_json_returns_none_for_incomplete_fields() {
    let body = r#"{"outcome":"done"}"#;
    assert!(extract_task_execution_report_json(body).is_none());
}

// ── 310bc8d: stale_claim_recovery_status ──────────────────────

fn make_test_task() -> CoordinationTask {
    CoordinationTask {
        task_id: "task-1".to_string(),
        run_id: "run-1".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: None,
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "test task".to_string(),
        prompt: "do something".to_string(),
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
        max_attempts: DEFAULT_TASK_MAX_ATTEMPTS,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::GeneratedWriteTask,
        )),
        status: CoordinationTaskStatus::Running,
        claimed_by: Some(1234),
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn stale_claim_requeues_when_attempts_remain() {
    let mut task = make_test_task();
    task.attempt_count = 1;
    task.max_attempts = 3;
    assert_eq!(
        stale_claim_recovery_status(&task),
        CoordinationTaskStatus::Queued
    );
}

#[test]
fn stale_claim_requeues_when_max_attempts_exhausted() {
    let mut task = make_test_task();
    task.attempt_count = 3;
    task.max_attempts = 3;
    assert_eq!(
        stale_claim_recovery_status(&task),
        CoordinationTaskStatus::Queued
    );
}

#[test]
fn stale_claim_requeues_when_attempts_exceed_max() {
    let mut task = make_test_task();
    task.attempt_count = 5;
    task.max_attempts = 3;
    assert_eq!(
        stale_claim_recovery_status(&task),
        CoordinationTaskStatus::Queued
    );
}

#[test]
fn stale_claim_requeues_with_zero_max_attempts() {
    let mut task = make_test_task();
    task.attempt_count = 0;
    task.max_attempts = 0;
    assert_eq!(
        stale_claim_recovery_status(&task),
        CoordinationTaskStatus::Queued
    );
}

// ── 8e99b01: provider cooldown pruning ───────────────────────────

#[test]
fn prune_expired_cooldowns_removes_past_entries() {
    let now = Utc::now();
    let mut cooldowns = BTreeMap::new();
    cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now - chrono::Duration::seconds(60),
            reason: "quota exhausted".to_string(),
        },
    );
    cooldowns.insert(
        "claude".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::seconds(300),
            reason: "rate limited".to_string(),
        },
    );

    let changed = prune_expired_provider_cooldown_map(&mut cooldowns, now);
    assert!(changed);
    assert!(!cooldowns.contains_key("gemini"));
    assert!(cooldowns.contains_key("claude"));
}

#[test]
fn prune_expired_cooldowns_returns_false_when_nothing_expired() {
    let now = Utc::now();
    let mut cooldowns = BTreeMap::new();
    cooldowns.insert(
        "claude".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::seconds(300),
            reason: "rate limited".to_string(),
        },
    );

    let changed = prune_expired_provider_cooldown_map(&mut cooldowns, now);
    assert!(!changed);
}

#[test]
fn upsert_cooldown_extends_but_does_not_shorten() {
    let now = Utc::now();
    let mut cooldowns = BTreeMap::new();

    let long = ProviderCooldown {
        unavailable_until: now + chrono::Duration::seconds(600),
        reason: "long".to_string(),
    };
    let short = ProviderCooldown {
        unavailable_until: now + chrono::Duration::seconds(60),
        reason: "short".to_string(),
    };

    assert!(upsert_provider_cooldown(&mut cooldowns, "gemini", &long));
    assert!(!upsert_provider_cooldown(&mut cooldowns, "gemini", &short));
    assert_eq!(cooldowns["gemini"].reason, "long");
}

#[test]
fn merge_cooldowns_combines_maps() {
    let now = Utc::now();
    let mut dest = BTreeMap::new();
    let mut source = BTreeMap::new();

    dest.insert(
        "claude".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::seconds(100),
            reason: "a".to_string(),
        },
    );
    source.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::seconds(200),
            reason: "b".to_string(),
        },
    );

    let changed = merge_provider_cooldowns(&mut dest, &source);
    assert!(changed);
    assert!(dest.contains_key("claude"));
    assert!(dest.contains_key("gemini"));
}

#[test]
fn select_fallback_skips_cooled_down_providers() {
    let now = Utc::now();
    let mut run = CoordinationRun {
        run_id: "run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "claude".to_string(),
        goal: "test".to_string(),
        workflow_mode: AutonomousWorkflowMode::SingleProvider,
        design_doc_path: String::new(),
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
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    // Cool down gemini
    run.provider_cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::seconds(600),
            reason: "quota".to_string(),
        },
    );

    let fallback = select_fallback_provider(&run, "gemini", now);
    assert!(fallback.is_some());
    assert_ne!(fallback.as_deref(), Some("gemini"));
}

// ── prepare_task_for_manual_restart ───────────────────────────────

#[test]
fn manual_restart_resets_failed_task() {
    let mut task = make_test_task();
    task.status = CoordinationTaskStatus::Failed;
    task.attempt_count = 3;
    task.max_attempts = 3;

    let restarted =
        prepare_task_for_manual_restart(&mut task, Utc::now(), default_execution_generation() + 1);
    assert_eq!(restarted, Some(default_execution_generation()));
    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert!(task.claimed_by.is_none());
    // max_attempts should be bumped so the task can actually run again
    assert!(task.max_attempts > task.attempt_count);
}

#[test]
fn manual_restart_ignores_running_task() {
    let mut task = make_test_task();
    task.status = CoordinationTaskStatus::Running;

    let restarted =
        prepare_task_for_manual_restart(&mut task, Utc::now(), default_execution_generation() + 1);
    assert!(restarted.is_none());
    assert_eq!(task.status, CoordinationTaskStatus::Running);
}

// ── json_has_required_report_fields ───────────────────────────────

#[test]
fn json_has_required_fields_with_all_present() {
    assert!(json_has_required_report_fields(
        r#"{"outcome":"done","summary":"ok","details":"checked"}"#
    ));
}

#[test]
fn json_has_required_fields_missing_outcome() {
    assert!(!json_has_required_report_fields(
        r#"{"summary":"ok","details":"checked"}"#
    ));
}

#[test]
fn json_has_required_fields_invalid_json() {
    assert!(!json_has_required_report_fields("{not json}"));
}

// ── normalize_path_list / normalize_string_list ───────────────────

#[test]
fn normalize_path_list_strips_backticks_and_whitespace() {
    let input = vec![
        "  `src/lib.rs`  ".to_string(),
        "".to_string(),
        "  tests/main.rs  ".to_string(),
    ];
    let result = normalize_path_list(input);
    assert_eq!(result, vec!["src/lib.rs", "tests/main.rs"]);
}

#[test]
fn normalize_string_list_filters_empty() {
    let input = vec![
        "  hello  ".to_string(),
        "  ".to_string(),
        "world".to_string(),
    ];
    let result = normalize_string_list(input);
    assert_eq!(result, vec!["hello", "world"]);
}

// ── wildcard_prefix (used in scope matching) ─────────────────────

#[test]
fn wildcard_prefix_extracts_before_star() {
    assert_eq!(wildcard_prefix("src/*"), Some("src".to_string()));
}

#[test]
fn wildcard_prefix_returns_none_without_star() {
    assert_eq!(wildcard_prefix("src/lib.rs"), None);
}

// ── restart_failed_tasks (integration) ──────────────────────────────

fn make_restart_test_task(
    task_id: &str,
    run_id: &str,
    status: CoordinationTaskStatus,
    now: DateTime<Utc>,
) -> CoordinationTask {
    CoordinationTask {
        task_id: task_id.to_string(),
        run_id: run_id.to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: format!("Task {task_id}"),
        prompt: "Do work".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: vec!["src/lib.rs".to_string()],
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 1,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::GeneratedWriteTask,
        )),
        status,
        claimed_by: if status == CoordinationTaskStatus::Running {
            Some(1)
        } else {
            None
        },
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn manual_restart_only_affects_failed_and_canceled_not_completed() {
    let now = Utc::now();

    let mut failed = make_restart_test_task("t1", "run", CoordinationTaskStatus::Failed, now);
    let mut canceled = make_restart_test_task("t2", "run", CoordinationTaskStatus::Canceled, now);
    let mut completed = make_restart_test_task("t3", "run", CoordinationTaskStatus::Completed, now);
    let mut queued = make_restart_test_task("t4", "run", CoordinationTaskStatus::Queued, now);
    let mut running = make_restart_test_task("t5", "run", CoordinationTaskStatus::Running, now);

    // Failed and Canceled should be restarted
    assert!(prepare_task_for_manual_restart(&mut failed, now, 2).is_some());
    assert_eq!(failed.status, CoordinationTaskStatus::Queued);

    assert!(prepare_task_for_manual_restart(&mut canceled, now, 2).is_some());
    assert_eq!(canceled.status, CoordinationTaskStatus::Queued);

    // Completed, Queued, and Running should NOT be restarted
    assert!(prepare_task_for_manual_restart(&mut completed, now, 2).is_none());
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);

    assert!(prepare_task_for_manual_restart(&mut queued, now, 2).is_none());
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);

    assert!(prepare_task_for_manual_restart(&mut running, now, 2).is_none());
    assert_eq!(running.status, CoordinationTaskStatus::Running);
}

#[test]
fn manual_restart_preserves_successful_tasks_in_mixed_run() {
    let now = Utc::now();

    // Simulate a run with mixed outcomes: 2 completed, 1 failed, 1 canceled
    let mut tasks = vec![
        make_restart_test_task("t1", "run", CoordinationTaskStatus::Completed, now),
        make_restart_test_task("t2", "run", CoordinationTaskStatus::Completed, now),
        make_restart_test_task("t3", "run", CoordinationTaskStatus::Failed, now),
        make_restart_test_task("t4", "run", CoordinationTaskStatus::Canceled, now),
    ];

    let mut restarted = 0usize;
    for task in tasks.iter_mut() {
        if prepare_task_for_manual_restart(task, now, 2).is_some() {
            restarted += 1;
        }
    }

    // Only the 2 non-completed tasks should restart
    assert_eq!(restarted, 2);

    // Completed tasks remain untouched
    assert_eq!(tasks[0].status, CoordinationTaskStatus::Completed);
    assert_eq!(tasks[1].status, CoordinationTaskStatus::Completed);

    // Failed/canceled tasks are now queued
    assert_eq!(tasks[2].status, CoordinationTaskStatus::Queued);
    assert_eq!(tasks[3].status, CoordinationTaskStatus::Queued);
}

#[test]
fn manual_restart_clears_claim_and_artifact_state() {
    let now = Utc::now();
    let mut task = make_restart_test_task("t1", "run", CoordinationTaskStatus::Failed, now);
    task.claimed_by = Some(42);
    task.artifact_summary = Some("error: OOM".to_string());
    task.attempt_count = 2;
    task.max_attempts = 2;

    assert!(prepare_task_for_manual_restart(&mut task, now, 2).is_some());

    assert!(task.claimed_by.is_none());
    assert!(task.artifact_summary.is_none());
    // max_attempts should be extended by two since attempt_count == max_attempts
    assert_eq!(task.max_attempts, 4);
}

#[test]
fn manual_restart_grants_two_additional_attempts_when_exhausted() {
    let now = Utc::now();
    let mut task = make_restart_test_task("t1", "run", CoordinationTaskStatus::Failed, now);
    task.attempt_count = 3;
    task.max_attempts = 3;

    assert!(prepare_task_for_manual_restart(&mut task, now, 2).is_some());

    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert_eq!(task.attempt_count, 3);
    assert_eq!(task.max_attempts, 5);
}

#[test]
fn unblock_runnable_tasks_rebases_blocked_task_to_completed_dependency_generation() {
    let now = Utc::now();
    let mut dependency_a =
        make_restart_test_task("dep-a", "run", CoordinationTaskStatus::Completed, now);
    dependency_a.execution_generation = 2;
    let mut dependency_b =
        make_restart_test_task("dep-b", "run", CoordinationTaskStatus::Completed, now);
    dependency_b.execution_generation = 2;
    let mut blocked =
        make_restart_test_task("blocked", "run", CoordinationTaskStatus::Blocked, now);
    blocked.execution_generation = 1;
    blocked.dependencies = vec![dependency_a.task_id.clone(), dependency_b.task_id.clone()];

    let mut tasks = vec![dependency_a, dependency_b, blocked];
    let changed = unblock_runnable_task_ids(&mut tasks);
    let blocked = tasks.iter().find(|task| task.task_id == "blocked").unwrap();

    assert_eq!(blocked.execution_generation, 1);
    assert_eq!(blocked.dependency_generation, Some(2));
    assert_eq!(blocked.status, CoordinationTaskStatus::Queued);
    assert!(changed.contains(&"blocked".to_string()));
}

#[test]
fn reconcile_run_state_rebases_dependency_drift_and_logs_issue() {
    let fixture = CoordinationTestFixture::named("reconcile-dependency-drift");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let now = Utc::now();
    let mut dependency =
        make_restart_test_task("dep", &run.run_id, CoordinationTaskStatus::Completed, now);
    dependency.execution_generation = 2;
    dependency.dependency_generation = Some(2);
    let mut blocked =
        make_restart_test_task("blocked", &run.run_id, CoordinationTaskStatus::Blocked, now);
    blocked.execution_generation = 1;
    blocked.dependencies = vec![dependency.task_id.clone()];

    fixture.persist_run_with_tasks(&run, &[dependency.clone(), blocked.clone()]);
    reconcile_run_state(&run.run_id).unwrap();

    let blocked = load_task(&run.run_id, "blocked").unwrap();
    assert_eq!(blocked.dependency_generation, Some(2));
    assert_eq!(blocked.status, CoordinationTaskStatus::Queued);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "run_reconcile_issue_detected"
            && event.message.contains("dependency generation drift")
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "run_reconcile_issue_repaired"
            && event.message.contains("Rebased dependency generation")
    }));
}

#[test]
fn replay_incomplete_complete_task_batch_recovers_completed_task_state() {
    let fixture = CoordinationTestFixture::named("replay-complete-task-batch");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut task = fixture.make_task(
        &run,
        "write-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "implemented slice".to_string(),
        body: "implemented slice".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "implemented slice".to_string(),
            details: "implemented slice".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            tests_run: Vec::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
            remaining_gaps: Vec::new(),
            design_requirements_satisfied: Vec::new(),
            validation_errors: Vec::new(),
            failure_kind: None,
            recovery_action: None,
            fix_tasks: Vec::new(),
        },
    });
    let batch = begin_mutation_batch(
        &run.run_id,
        Some(&task.task_id),
        CoordinationMutationKind::CompleteTask,
        "Simulate crash during completion.",
    )
    .unwrap();

    replay_incomplete_mutation_batches_for_run(&run.run_id).unwrap();

    let recovered = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(recovered.status, CoordinationTaskStatus::Completed);
    assert_eq!(
        recovered.recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );

    let batch_raw =
        std::fs::read_to_string(mutation_batch_path(&run.run_id, &batch.batch_id)).unwrap();
    let replayed: CoordinationMutationBatch = serde_json::from_str(&batch_raw).unwrap();
    assert!(replayed.completed_at.is_some());
}

#[test]
fn reconcile_run_state_updates_run_status_to_active_when_only_completed_tasks_carry_recovery_metadata(
) {
    let fixture = CoordinationTestFixture::named("reconcile-run-status-active");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.status = CoordinationRunStatus::Recovering;
    let now = Utc::now();

    let mut repaired_completed = make_restart_test_task(
        "repaired",
        &run.run_id,
        CoordinationTaskStatus::Completed,
        now,
    );
    repaired_completed.failure_kind = Some(TaskFailureKind::DependencyDrift);
    repaired_completed.recovery_action = Some(TaskRecoveryAction::ReconcileState);

    let running =
        make_restart_test_task("running", &run.run_id, CoordinationTaskStatus::Queued, now);

    fixture.persist_run_with_tasks(&run, &[repaired_completed, running]);
    reconcile_run_state(&run.run_id).unwrap();

    let reloaded = load_run(&run.run_id).unwrap();
    assert_eq!(reloaded.status, CoordinationRunStatus::Active);
}

#[test]
fn background_driver_considers_recovering_runs_drivable() {
    let row = RunSummaryRow {
        run_id: "run".to_string(),
        goal: "goal".to_string(),
        provider: "codex".to_string(),
        status: "recovering".to_string(),
        workflow_mode: Some(
            AutonomousWorkflowMode::LightweightImplementation
                .as_str()
                .to_string(),
        ),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };

    assert!(crate::driver::should_drive_run(&row));
}

#[test]
fn replay_incomplete_mutation_batches_silently_completes_reconcile_run_batches() {
    let fixture = CoordinationTestFixture::named("reconcile-replay-silent");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);

    let batch = begin_mutation_batch(
        &run.run_id,
        None,
        CoordinationMutationKind::ReconcileRun,
        "Simulate interrupted reconcile.",
    )
    .unwrap();

    let changed = replay_incomplete_mutation_batches_for_run(&run.run_id).unwrap();
    assert!(!changed);

    let raw = fs::read_to_string(mutation_batch_path(&run.run_id, &batch.batch_id)).unwrap();
    let replayed: CoordinationMutationBatch = serde_json::from_str(&raw).unwrap();
    assert!(replayed.completed_at.is_some());

    assert!(!events_path(&run.run_id).is_file());
}

// ── 238f69f: future created_at mutation batch skips instead of looping ─

#[test]
fn load_incomplete_mutation_batches_skips_future_created_at_batch() {
    use chrono::Duration;

    let fixture = CoordinationTestFixture::named("future-batch-skip");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);

    // A batch with a future created_at (simulates crash between begin and complete)
    let future_time = Utc::now() + Duration::days(1);
    let mut future_batch = CoordinationMutationBatch {
        batch_id: Uuid::new_v4().to_string(),
        run_id: run.run_id.clone(),
        task_id: Some("task-1".to_string()),
        kind: CoordinationMutationKind::CompleteTask,
        created_at: future_time,
        completed_at: None,
        notes: "Simulated crash batch".to_string(),
    };

    // Write the batch file directly (mimics a crash leaving it incomplete)
    write_json_atomic(
        &mutation_batch_path(&run.run_id, &future_batch.batch_id),
        &future_batch,
    )
    .unwrap();

    // Also write a normal incomplete batch (should still be returned)
    let normal_batch = begin_mutation_batch(
        &run.run_id,
        Some("task-2"),
        CoordinationMutationKind::CompleteTask,
        "Normal batch",
    )
    .unwrap();

    // load_incomplete_mutation_batches should NOT return the future batch
    let processed = BTreeSet::new();
    let batches = load_incomplete_mutation_batches(&run.run_id, &processed).unwrap();

    // Normal batch should be present, future batch should NOT
    assert_eq!(batches.len(), 1, "Only normal batch should be returned");
    assert_eq!(batches[0].batch_id, normal_batch.batch_id);

    // The future batch file should now be marked complete (auto-completed by loader)
    let future_raw =
        fs::read_to_string(mutation_batch_path(&run.run_id, &future_batch.batch_id)).unwrap();
    let future_reloaded: CoordinationMutationBatch = serde_json::from_str(&future_raw).unwrap();
    assert!(
        future_reloaded.completed_at.is_some(),
        "Future batch should be auto-completed to prevent stuck replay loop"
    );

    // Verify normal batch is still incomplete (not touched)
    let normal_raw =
        fs::read_to_string(mutation_batch_path(&run.run_id, &normal_batch.batch_id)).unwrap();
    let normal_reloaded: CoordinationMutationBatch = serde_json::from_str(&normal_raw).unwrap();
    assert!(
        normal_reloaded.completed_at.is_none(),
        "Normal batch should remain incomplete"
    );

    // Verify a mutation_batch_replay_skipped event was logged
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event_type == "mutation_batch_replay_skipped"),
        "A skipped event should be logged for the future batch"
    );
}

#[test]
fn load_incomplete_mutation_batches_skips_future_completed_at_batch() {
    // Defensive check: if completed_at is in the future, batch should also be skipped
    let fixture = CoordinationTestFixture::named("future-completed-at-skip");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);

    let future_time = Utc::now() + chrono::Duration::hours(1);
    let batch = CoordinationMutationBatch {
        batch_id: Uuid::new_v4().to_string(),
        run_id: run.run_id.clone(),
        task_id: Some("task-1".to_string()),
        kind: CoordinationMutationKind::CompleteTask,
        created_at: Utc::now(),
        completed_at: Some(future_time), // completed_at in future is also anomalous
        notes: "Tampered batch file".to_string(),
    };

    write_json_atomic(&mutation_batch_path(&run.run_id, &batch.batch_id), &batch).unwrap();

    let processed = BTreeSet::new();
    let batches = load_incomplete_mutation_batches(&run.run_id, &processed).unwrap();

    // Should be filtered out (even though it has completed_at set)
    assert!(
        batches.is_empty(),
        "Future completed_at batch should be filtered out"
    );
}

#[test]
fn status_projection_ignores_non_lightweight_workflows_without_creating_snapshots() {
    for mode in [
        AutonomousWorkflowMode::DesignConsensus,
        AutonomousWorkflowMode::GithubBranchFix,
        AutonomousWorkflowMode::FreeformCouncil,
    ] {
        let fixture = CoordinationTestFixture::named("status-projection-mode-boundary");
        let run = fixture.make_run(mode);
        fixture.persist_run_with_tasks(&run, &[]);
        refresh_lightweight_status_projection(&run.run_id)
            .expect("non-lightweight lifecycle updates must not require a lightweight broker");
        assert!(!lightweight_broker_snapshot_path(&run.run_id).exists());
        assert!(refresh_lightweight_broker_snapshot(&run.run_id).is_err());
    }
}
