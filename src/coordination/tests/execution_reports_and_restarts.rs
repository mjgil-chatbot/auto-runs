#[test]
fn extracts_first_json_object_from_mixed_output() {
    let body = "notes before\n{\"complete\":false,\"reason\":\"missing test\"}\nnotes after";
    assert_eq!(
        extract_first_json_object(body),
        Some("{\"complete\":false,\"reason\":\"missing test\"}")
    );
}

#[test]
fn parses_ralph_verification_reason() {
    let parsed = parse_ralph_verification(
        "```json\n{\"complete\": true, \"reason\": \"all criteria satisfied\"}\n```",
        "test",
    )
    .expect("verification should parse");

    assert!(parsed.complete);
    assert_eq!(parsed.reason, "all criteria satisfied");
}

#[test]
fn collect_stream_response_keeps_non_empty_final_json() {
    let (tx, rx) = mpsc::channel();
    tx.send(crate::stream::StreamMessage {
        response: "{\"complete\":false,\"reason\":\"missing tests\"}".to_string(),
        command: "codex exec".to_string(),
        session_id: None,
        summary: None,
        is_final: false,
    })
    .expect("first response should send");
    tx.send(crate::stream::StreamMessage {
        response: String::new(),
        command: "codex exec".to_string(),
        session_id: None,
        summary: None,
        is_final: true,
    })
    .expect("empty final response should send");
    drop(tx);

    assert_eq!(
        collect_stream_response(rx),
        "{\"complete\":false,\"reason\":\"missing tests\"}"
    );
}

#[test]
fn detects_provider_command_failure_response() {
    assert_eq!(
        extract_provider_command_failure("Gemini command failed.\nRecent stderr output:\nboom"),
        Some("Gemini command failed.\nRecent stderr output:\nboom")
    );
    assert_eq!(
        extract_provider_command_failure("qwen-code command failed: oauth expired"),
        Some("qwen-code command failed: oauth expired")
    );
    assert_eq!(
        extract_provider_command_failure(
            "Failed to spawn qwen-code command: No such file or directory (os error 2)"
        ),
        Some("Failed to spawn qwen-code command: No such file or directory (os error 2)")
    );
    assert_eq!(
        extract_provider_command_failure("{\"outcome\":\"completed_no_change\"}"),
        None
    );
}

#[test]
fn provider_task_timeout_uses_task_estimate_and_provider_cap() {
    let write_task = CoordinationTask {
        kind: CoordinationTaskKind::GeneratedWriteTask,
        estimated_minutes: Some(12),
        ..CoordinationTask::default()
    };
    assert_eq!(
        provider_task_timeout("codex", &write_task),
        Some(Duration::from_secs(12 * 60))
    );
    assert_eq!(
        provider_task_timeout("qwen", &write_task),
        Some(Duration::from_secs(12 * 60))
    );

    let long_task = CoordinationTask {
        kind: CoordinationTaskKind::GeneratedWriteTask,
        estimated_minutes: Some(45),
        ..CoordinationTask::default()
    };
    assert_eq!(
        provider_task_timeout("qwen", &long_task),
        Some(Duration::from_secs(20 * 60))
    );
    assert_eq!(
        provider_timeout_failure("qwen", Duration::from_secs(20 * 60)),
        "Qwen task timed out after 20 minutes."
    );
}

#[test]
fn parses_perf_parse_freeform_brainstorm_output() {
    let body = r#"
## Speed Improvement Brainstorm

### High-Impact Ideas

**1. Trust `d_type` to skip stat calls**
- On ext4/xfs/btrfs, `d_type` reliably tells you if something is a file or dir.
- Currently every entry gets `fstatat()` regardless.

**2. Incremental hardlink deduplication**
- Track seen `(inode, device)` pairs during the scan itself.
- Eliminate the post-scan full-tree dedup pass.
"#;

    let ideas = parse_perf_ideas(body).expect("freeform brainstorm should be salvaged");

    assert_eq!(ideas.ideas.len(), 2);
    assert_eq!(ideas.ideas[0].index, 0);
    assert_eq!(ideas.ideas[0].title, "Trust `d_type` to skip stat calls");
    assert!(ideas.ideas[0].description.contains("fstatat()"));
    assert_eq!(ideas.ideas[1].index, 1);
    assert!(ideas.ideas[1]
        .description
        .contains("post-scan full-tree dedup pass"));
}

#[test]
fn perf_parse_report_succeeds_with_markdown_heading_format() {
    let body = r#"
### 1. **Reduce Syscall Overhead**
- Use `getdents64` directly to reduce per-entry libc overhead.

### 2. **Adaptive Parallelism**
- Tune subtree spawning based on queue saturation instead of fixed depth.
"#;

    let report = perf_parse_report(body).expect("markdown heading output should parse");

    assert_eq!(report.summary, "Parsed 2 performance improvement ideas");
    assert_eq!(
        report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
}

#[test]
fn parses_structured_task_execution_report() {
    let report = parse_task_execution_report(
            "{\"outcome\":\"completed_with_changes\",\"summary\":\"wired backend\",\"details\":\"done\",\"changed_files\":[\"src/backend.rs\"],\"tests_run\":[\"cargo test\"],\"warnings\":[],\"blockers\":[],\"remaining_gaps\":[],\"design_requirements_satisfied\":[\"backend wired\"]}",
        )
        .expect("report should parse");

    assert_eq!(
        report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert_eq!(report.summary, "wired backend");
    assert_eq!(report.changed_files, vec!["src/backend.rs"]);
}

#[test]
fn parses_task_execution_report_after_non_report_json_object() {
    let report = parse_task_execution_report(
            "Request body: {\"query_text\":\"abc\",\"limit\":10}\n{\"outcome\":\"completed_no_change\",\"summary\":\"validated workspace\",\"details\":\"No code changes were required.\",\"changed_files\":[],\"tests_run\":[],\"warnings\":[],\"blockers\":[],\"remaining_gaps\":[],\"design_requirements_satisfied\":[\"checked response shape\"]}",
        )
        .expect("report should parse after earlier JSON object");

    assert_eq!(report.outcome, CoordinationTaskOutcome::CompletedNoChange);
    assert_eq!(report.summary, "validated workspace");
}

#[test]
fn rejects_task_execution_report_without_required_top_level_fields() {
    let err = parse_task_execution_report(
            "{\"warnings\":[\"mentions \\\"outcome\\\"\"],\"blockers\":[\"mentions \\\"summary\\\"\"],\"remaining_gaps\":[\"mentions \\\"details\\\"\"]}",
        )
        .expect_err("missing required top-level fields should be rejected");

    assert!(err
        .to_string()
        .contains("missing JSON object in task output"));
}

#[test]
fn build_review_plan_merges_scope_warnings_into_chunk_metadata() {
    let workspace =
        std::env::temp_dir().join(format!("rust-chatbot-review-warnings-{}", Uuid::new_v4()));
    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/lib.rs"), "pub fn example() {}\n").unwrap();

    let plan = code_review::build_review_plan(
        &workspace,
        &ReviewScope::Globs(vec![
            "src/**/*.rs".to_string(),
            "missing/**/*.rs".to_string(),
        ]),
        None,
        ReviewFixMode::ReportOnly,
        Vec::new(),
        vec!["profile warning".to_string()],
        Utc::now(),
    )
    .expect("review plan should build");

    let metadata = plan.review_tasks[0]
        .metadata
        .as_ref()
        .expect("review task metadata should exist");
    assert!(metadata
        .profile_warnings
        .iter()
        .any(|warning| warning.contains("missing/**/*.rs")));
    assert!(metadata
        .profile_warnings
        .iter()
        .any(|warning| warning == "profile warning"));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn parses_retry_delay_ms_from_gemini_quota_error() {
    let response =
        "Gemini command failed.\nretryDelayMs: 75675110.700098,\nreason: 'QUOTA_EXHAUSTED'";
    assert_eq!(extract_retry_delay_ms(response), Some(75_675_111));
}

#[test]
fn extracts_gemini_provider_cooldown() {
    let now = Utc::now();
    let response = "Gemini command failed.\nError when talking to Gemini API.\nTerminalQuotaError: You have exhausted your capacity on this model. Your quota will reset after 21h1m15s.\nretryDelayMs: 75675110.700098,\nreason: 'QUOTA_EXHAUSTED'";
    let cooldown =
        extract_provider_cooldown("gemini", response, now).expect("cooldown should parse");

    assert_eq!(cooldown.reason, "Gemini API quota exhausted.");
    assert!(cooldown.unavailable_until > now);
}

#[test]
fn extracts_gemini_provider_cooldown_from_docker_capacity_errors() {
    let now = Utc::now();
    let response = "Gemini command failed.\nRecent stderr output:\nstatus: 429\nstatusText: 'Too Many Requests'\nAttempt 5 failed with status 429. Retrying with backoff...\n{\"error\":{\"status\":\"RESOURCE_EXHAUSTED\",\"details\":[{\"reason\":\"MODEL_CAPACITY_EXHAUSTED\"}]}}";
    let cooldown =
        extract_provider_cooldown("gemini", response, now).expect("cooldown should parse");

    assert_eq!(cooldown.reason, "Gemini API quota exhausted.");
    assert_eq!(
        cooldown.unavailable_until - now,
        chrono::Duration::minutes(5)
    );
}

#[test]
fn extracts_glm_provider_cooldown() {
    let now = Utc::now();
    let response = "Claude command failed: API Error: 429 {\"error\":{\"code\":\"1308\",\"message\":\"已达到 5 小时的使用上限。您的限额将在 2099-12-31 23:59:59 重置。\"},\"request_id\":\"20260408015107f96597ccc364431a\"}";
    let cooldown = extract_provider_cooldown("glm", response, now).expect("cooldown should parse");

    assert_eq!(cooldown.reason, "GLM API quota exhausted.");
    assert_eq!(
        cooldown.unavailable_until.to_rfc3339(),
        "2099-12-31T15:59:59+00:00"
    );
}

#[test]
fn reroutes_claim_away_from_provider_on_cooldown() {
    let now = Utc::now();
    let mut cooldowns = BTreeMap::new();
    cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::hours(1),
            reason: "Gemini API quota exhausted.".to_string(),
        },
    );
    let run = CoordinationRun {
        run_id: "run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "gemini".to_string(),
        goal: "goal".to_string(),
        workflow_mode: AutonomousWorkflowMode::SingleProvider,
        design_doc_path: "docs/autonomous/design-doc.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: now,
        updated_at: now,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: String::new(),
        council_turn_policy: None,
        council_max_turns: None,
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        provider_cooldowns: cooldowns,
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };

    assert_eq!(
        resolve_provider_for_task_claim(&run, Some("gemini"), "gemini", now),
        Some("claude".to_string())
    );
}

#[test]
fn merges_global_provider_cooldown_into_run_state() {
    let now = Utc::now();
    let mut run_cooldowns = BTreeMap::new();
    let mut global_cooldowns = BTreeMap::new();
    global_cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::hours(1),
            reason: "Gemini API quota exhausted.".to_string(),
        },
    );

    assert!(merge_provider_cooldowns(
        &mut run_cooldowns,
        &global_cooldowns
    ));
    assert!(run_cooldowns.contains_key("gemini"));
}

#[test]
fn normalizes_leading_slash_in_write_scope_paths() {
    assert_eq!(
        normalize_scope("/docs/chat-autonomous.md"),
        "docs/chat-autonomous.md"
    );
    assert!(scope_allows_relative_path(
        "/docs/chat-autonomous.md",
        "docs/chat-autonomous.md"
    ));
}

#[test]
fn manual_restart_requeues_failed_task_and_extends_attempt_budget() {
    let now = Utc::now();
    let mut task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: vec!["src/lib.rs".to_string()],
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 3,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Failed,
        claimed_by: Some(42),
        artifact_summary: Some("previous failure".to_string()),
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    assert!(prepare_task_for_manual_restart(&mut task, now, 2).is_some());
    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert_eq!(task.max_attempts, 5);
    assert!(task.claimed_by.is_none());
    assert!(task.artifact_summary.is_none());
}

#[test]
fn manual_restart_requeues_canceled_task() {
    let now = Utc::now();
    let mut task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
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
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Canceled,
        claimed_by: Some(42),
        artifact_summary: Some("stopped by user".to_string()),
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    assert!(prepare_task_for_manual_restart(&mut task, now, 2).is_some());
    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert!(task.claimed_by.is_none());
    assert!(task.artifact_summary.is_none());
}

#[test]
fn manual_restart_skips_completed_task() {
    let now = Utc::now();
    let mut task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
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
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Completed,
        claimed_by: None,
        artifact_summary: Some("done".to_string()),
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    assert!(prepare_task_for_manual_restart(&mut task, now, 2).is_none());
    assert_eq!(task.status, CoordinationTaskStatus::Completed);
}

#[test]
fn stale_claim_requeues_even_when_attempt_budget_is_exhausted() {
    let retryable_task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
        cwd: ".".to_string(),
        allowed_tools: Vec::new(),
        read_scope: Vec::new(),
        write_scope: vec!["src/lib.rs".to_string()],
        acceptance_checks: Vec::new(),
        review_metadata: None,
        dependencies: Vec::new(),
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 2,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Running,
        claimed_by: Some(42),
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let exhausted_task = CoordinationTask {
        attempt_count: 5,
        max_attempts: 5,
        ..retryable_task.clone()
    };

    assert_eq!(
        stale_claim_recovery_status(&retryable_task),
        CoordinationTaskStatus::Queued
    );
    assert_eq!(
        stale_claim_recovery_status(&exhausted_task),
        CoordinationTaskStatus::Queued
    );
}

#[test]
fn adopting_claim_rebinds_task_to_worker_pid() {
    let now = Utc::now();
    let mut claim = TaskClaim {
        run_id: "run".to_string(),
        task_id: "task".to_string(),
        provider: "codex".to_string(),
        instance_pid: 100,
        launcher_pid: Some(100),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    };
    let mut task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
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
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Running,
        claimed_by: Some(100),
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    adopt_claim_in_memory(&mut claim, &mut task, 100, 200, now).unwrap();

    assert_eq!(claim.instance_pid, 200);
    assert_eq!(claim.launcher_pid, Some(100));
    assert_eq!(claim.lifecycle_phase, ClaimLifecyclePhase::WorkerAdopted);
    assert_eq!(claim.worker_pid, Some(200));
    assert_eq!(claim.worker_adopted_at, Some(now));
    assert_eq!(task.claimed_by, Some(200));
}

#[test]
fn adopting_claim_rejects_unexpected_launcher_pid() {
    let now = Utc::now();
    let mut claim = TaskClaim {
        run_id: "run".to_string(),
        task_id: "task".to_string(),
        provider: "codex".to_string(),
        instance_pid: 111,
        launcher_pid: Some(111),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    };
    let mut task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        parent_task_id: None,
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: None,
        provider_hint: None,
        required_role: None,
        iteration: None,
        title: "Implement feature".to_string(),
        prompt: "Prompt".to_string(),
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
        estimated_minutes: Some(15),
        status: CoordinationTaskStatus::Running,
        claimed_by: Some(111),
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    };

    let err = adopt_claim_in_memory(&mut claim, &mut task, 222, 333, now).unwrap_err();
    assert!(err.to_string().contains("not launcher 222"));
    assert_eq!(claim.instance_pid, 111);
    assert_eq!(task.claimed_by, Some(111));
}

#[test]
fn design_implementation_seed_uses_canonical_doc_pipeline() {
    let run = CoordinationRun {
        run_id: "run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "implement the canonical design".to_string(),
        workflow_mode: AutonomousWorkflowMode::DesignImplementation,
        design_doc_path: "docs/autonomous/design-doc.md".to_string(),
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

    let tasks = seed_initial_tasks(&run);
    assert_eq!(tasks.len(), 4);
    assert_eq!(tasks[0].kind, CoordinationTaskKind::SurveyWorkspace);
    assert_eq!(tasks[1].kind, CoordinationTaskKind::PlanFlatTasks);
    assert!(tasks[1].prompt.contains("canonical design doc"));
    assert!(tasks[1].prompt.contains(&run.design_doc_path));
    assert_eq!(tasks[2].kind, CoordinationTaskKind::ReviewArtifacts);
    assert!(tasks[2].read_scope.contains(&run.design_doc_path));
    assert_eq!(tasks[3].kind, CoordinationTaskKind::ApplyApprovedEdits);
}

#[test]
fn ralph_loop_seed_starts_with_plan_and_threads_it_through_iteration_one() {
    let run = CoordinationRun {
        run_id: "run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: ".".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "finish the feature".to_string(),
        workflow_mode: AutonomousWorkflowMode::RalphLoop,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: "- criterion one\n- criterion two".to_string(),
        max_iterations: 3,
        status: CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: "keep the fix narrow".to_string(),
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
    assert_eq!(tasks.len(), 4);
    assert_eq!(tasks[0].kind, CoordinationTaskKind::SurveyWorkspace);
    assert_eq!(tasks[1].kind, CoordinationTaskKind::RalphPlan);
    assert_eq!(tasks[1].dependencies, vec![tasks[0].task_id.clone()]);
    assert_eq!(tasks[2].kind, CoordinationTaskKind::RalphExecute);
    assert_eq!(tasks[2].dependencies, vec![tasks[1].task_id.clone()]);
    assert_eq!(tasks[3].kind, CoordinationTaskKind::RalphVerify);
    assert_eq!(
        tasks[3].dependencies,
        vec![tasks[1].task_id.clone(), tasks[2].task_id.clone()]
    );
}

#[test]
fn auto_retry_requires_remaining_attempts() {
    let run = CoordinationRun {
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
        auto_retry_failed_tasks: true,
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
    let retryable_task = CoordinationTask {
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
        attempt_count: 1,
        max_attempts: 3,
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::DraftDesignDoc,
        )),
        status: CoordinationTaskStatus::Failed,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let exhausted_task = CoordinationTask {
        attempt_count: 3,
        ..retryable_task.clone()
    };

    assert!(should_auto_retry_task(&run, &retryable_task, None));
    assert!(!should_auto_retry_task(&run, &exhausted_task, None));
}
