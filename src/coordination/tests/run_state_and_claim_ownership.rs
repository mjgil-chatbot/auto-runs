fn make_claim_for_test(run_id: &str, task_id: &str, instance_pid: u32, launcher_pid: u32) -> TaskClaim {
    let now = Utc::now();
    TaskClaim {
        run_id: run_id.to_string(),
        task_id: task_id.to_string(),
        provider: "codex".to_string(),
        instance_pid,
        launcher_pid: Some(launcher_pid),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    }
}

#[test]
fn write_presence_persists_agent_presence_file() {
    let fixture = CoordinationTestFixture::named("presence");

    let presence = write_presence(PresenceInput {
        provider: "codex".to_string(),
        instance_number: Some(2),
        instance_pid: 4242,
        current_load: 3,
        visible_workspaces: vec![fixture.workspace.path().display().to_string()],
    })
    .expect("presence write should succeed");

    assert_eq!(presence.provider, "codex");
    assert_eq!(presence.instance_number, Some(2));
    assert_eq!(presence.instance_pid, 4242);

    let raw = fs::read_to_string(presence_path(4242)).expect("presence file should exist");
    let persisted: AgentPresence =
        serde_json::from_str(&raw).expect("presence file should deserialize");
    assert_eq!(persisted.provider, "codex");
    assert_eq!(persisted.current_load, 3);
    assert_eq!(
        persisted.visible_workspaces,
        vec![fixture.workspace.path().display().to_string()]
    );
}

#[test]
fn claim_next_runnable_task_skips_tasks_when_required_role_is_missing() {
    let fixture = CoordinationTestFixture::named("required-role");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "role-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Role task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    task.required_role = Some(AgentRole::DeepWorker);
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let claimed = claim_next_runnable_task(&run.run_id, 777_777, "codex")
        .expect("claim should evaluate cleanly");

    assert!(claimed.is_none(), "task should not claim without matching role");
    assert_eq!(
        load_task(&run.run_id, &task.task_id)
            .expect("task should reload")
            .status,
        CoordinationTaskStatus::Queued
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn claim_next_runnable_task_prunes_cooldowns_and_reroutes_provider() {
    let fixture = CoordinationTestFixture::named("claim-reroute");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    run.provider_cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now - chrono::Duration::minutes(1),
            reason: "expired locally".to_string(),
        },
    );
    run.provider_cooldowns.insert(
        "claude".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::minutes(5),
            reason: "local cooldown".to_string(),
        },
    );
    let task = fixture.make_task(
        &run,
        "reroute-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("claude"),
        "Reroute task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let mut global_cooldowns = BTreeMap::new();
    global_cooldowns.insert(
        "minimax".to_string(),
        ProviderCooldown {
            unavailable_until: now - chrono::Duration::minutes(1),
            reason: "expired globally".to_string(),
        },
    );
    global_cooldowns.insert(
        "claude".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::minutes(6),
            reason: "quota".to_string(),
        },
    );
    write_global_provider_cooldowns(&global_cooldowns).expect("global cooldowns should write");

    let claimed = claim_next_runnable_task(&run.run_id, 91, "codex")
        .expect("claim should succeed")
        .expect("task should claim");

    assert_eq!(claimed.task_id, task.task_id);
    assert_ne!(claimed.assigned_provider.as_deref(), Some("claude"));
    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "task_claimed"
            && event
                .message
                .contains("rerouted to")
            && event.message.contains("claude is on provider cooldown")
    }));

    let reloaded_run = load_run(&run.run_id).expect("run should reload");
    assert!(!reloaded_run.provider_cooldowns.contains_key("gemini"));
    assert!(reloaded_run.provider_cooldowns.contains_key("claude"));
    let global = load_global_provider_cooldowns().expect("global cooldowns should reload");
    assert!(!global.contains_key("minimax"));
    assert!(global.contains_key("claude"));
}

#[test]
fn complete_task_rejects_wrong_claim_owner() {
    let fixture = CoordinationTestFixture::named("complete-owner");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "owned-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Owned task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(41);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&make_claim_for_test(&run.run_id, &task.task_id, 41, 41)).unwrap());

    let err = complete_task(
        &run.run_id,
        &task.task_id,
        99,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: None,
            task_category: None,
            fallback_from: None,
            summary: "done".to_string(),
            body: "details".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedNoChange,
                summary: "done".to_string(),
                details: "details".to_string(),
                ..Default::default()
            },
        },
    )
    .expect_err("wrong owner should fail");

    assert!(err.to_string().contains("owned by PID 41, not 99"));
}

#[test]
fn fail_task_rejects_wrong_claim_owner() {
    let fixture = CoordinationTestFixture::named("fail-owner");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "owned-failure",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Owned failure",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(51);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&make_claim_for_test(&run.run_id, &task.task_id, 51, 51)).unwrap());

    let err = fail_task(&run.run_id, &task.task_id, 52, "provider failed")
        .expect_err("wrong owner should fail");

    assert!(err.to_string().contains("owned by PID 51, not 52"));
}

#[test]
fn complete_task_validation_failure_requeues_retryable_task() {
    let fixture = CoordinationTestFixture::named("complete-validation-retry");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "validation-retry",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Validation retry",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(61);
    
    task.attempt_count = 1;
    task.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&make_claim_for_test(&run.run_id, &task.task_id, 61, 61)).unwrap());

    complete_task(
        &run.run_id,
        &task.task_id,
        61,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: None,
            task_category: None,
            fallback_from: None,
            summary: "changed files".to_string(),
            body: "details".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedNoChange,
                summary: "changed files".to_string(),
                details: "details".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                ..Default::default()
            },
        },
    )
    .expect("completion should fall back to retry path");

    let reloaded = load_task(&run.run_id, &task.task_id).expect("task should reload");
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let raw = fs::read_to_string(artifact_result_path(&run.run_id, &task.task_id))
        .expect("artifact should persist");
    let artifact: TaskArtifact = serde_json::from_str(&raw).expect("artifact should deserialize");
    assert!(!artifact.report.validation_errors.is_empty());

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| event.event_type == "task_retry_scheduled"));
    assert!(events.iter().any(|event| event.event_type == "task_validation_failed"));
}

#[test]
fn complete_task_explicit_failure_marks_task_failed() {
    let fixture = CoordinationTestFixture::named("complete-explicit-failure");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.auto_retry_failed_tasks = false;
    let mut task = fixture.make_task(
        &run,
        "explicit-failure",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Explicit failure",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(71);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&make_claim_for_test(&run.run_id, &task.task_id, 71, 71)).unwrap());

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: None,
            task_category: None,
            fallback_from: None,
            summary: "provider failed".to_string(),
            body: "details".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::Failed,
                summary: "provider failed".to_string(),
                details: "details".to_string(),
                ..Default::default()
            },
        },
    )
    .expect("completion should mark failure");

    let reloaded = load_task(&run.run_id, &task.task_id).expect("task should reload");
    assert_eq!(reloaded.status, CoordinationTaskStatus::Failed);
    assert_eq!(reloaded.claimed_by, Some(71));

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| event.event_type == "task_failed"));
}

#[test]
fn upsert_run_updates_existing_unseeded_run_configuration() {
    let fixture = CoordinationTestFixture::named("upsert-update");
    let mut existing = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    existing.notes = "before".to_string();
    write_json_atomic(&run_path(&existing.run_id), &existing).expect("run should persist");

    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: Some(existing.run_id.clone()),
        root_chat_id: existing.root_chat_id.clone(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "claude".to_string(),
        goal: "updated goal".to_string(),
        workflow_mode: AutonomousWorkflowMode::CodeReview,
        design_doc_path: "docs/updated.md".to_string(),
        completion_criteria: "updated criteria".to_string(),
        max_iterations: 1,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: false,
        require_review_before_apply: true,
        auto_retry_failed_tasks: false,
        notes: "after".to_string(),
        review_scope: Some(ReviewScope::Paths(vec!["src/lib.rs".to_string()])),
        review_dimensions: Some(vec!["architecture".to_string()]),
        review_fix_mode: Some(ReviewFixMode::GenerateFixes),
        council_turn_policy: Some(TurnPolicy::Adaptive),
        council_max_turns: Some(9),
    })
    .expect("update should succeed");

    assert_eq!(snapshot.run.goal, "updated goal");
    assert_eq!(snapshot.run.workflow_mode, AutonomousWorkflowMode::CodeReview);
    assert_eq!(snapshot.run.design_doc_path, "docs/updated.md");
    assert_eq!(snapshot.run.completion_criteria, "updated criteria");
    assert_eq!(snapshot.run.max_iterations, 1);
    assert_eq!(snapshot.run.max_agents, 1);
    assert!(!snapshot.run.allow_write_tasks);
    assert!(snapshot.run.require_review_before_apply);
    assert!(!snapshot.run.auto_retry_failed_tasks);
    assert_eq!(snapshot.run.notes, "after");
    assert_eq!(
        snapshot.run.review_scope,
        Some(ReviewScope::Paths(vec!["src/lib.rs".to_string()]))
    );
    assert_eq!(
        snapshot.run.review_dimensions,
        Some(vec!["architecture".to_string()])
    );
    assert_eq!(snapshot.run.review_fix_mode, Some(ReviewFixMode::GenerateFixes));
    assert_eq!(snapshot.run.council_turn_policy, Some(TurnPolicy::Adaptive));
    assert_eq!(snapshot.run.council_max_turns, Some(9));
    assert_eq!(snapshot.run.provider, existing.provider);

    let events = load_events(&existing.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| event.event_type == "run_updated"));
}

#[test]
fn upsert_run_creates_freeform_council_layout() {
    let fixture = CoordinationTestFixture::named("upsert-council");

    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "claude".to_string(),
        goal: "drive council".to_string(),
        workflow_mode: AutonomousWorkflowMode::FreeformCouncil,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: "ship it".to_string(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: "council notes".to_string(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("council run should create");

    assert_eq!(
        snapshot.run.workflow_mode,
        AutonomousWorkflowMode::FreeformCouncil
    );
    assert!(snapshot.tasks.is_empty());
    assert!(council_json_path(&snapshot.run.run_id).is_file());
    assert!(council_ledger_path(&snapshot.run.run_id).is_file());

    let raw = fs::read_to_string(council_json_path(&snapshot.run.run_id))
        .expect("council state should persist");
    let council: council::CouncilRun =
        serde_json::from_str(&raw).expect("council state should deserialize");
    assert_eq!(council.goal, "drive council");
    assert_eq!(council.notes, "council notes");

    let events = load_events(&snapshot.run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| event.event_type == "council_seeded"));
    assert!(events.iter().any(|event| {
        event.event_type == "run_created"
            && event.message.contains("Freeform Council")
    }));
}

#[test]
fn upsert_run_seeds_provider_registry_into_design_consensus_workspace() {
    let fixture = CoordinationTestFixture::named("upsert-design-registry");

    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "draft the design".to_string(),
        workflow_mode: AutonomousWorkflowMode::DesignConsensus,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        created_by_instance: None,
        max_agents: 2,
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
    .expect("design consensus run should create");

    let registry_path = fixture
        .workspace
        .path()
        .join("workflow-registry/providers.json");
    assert!(
        registry_path.is_file(),
        "design consensus runs should seed the provider registry into the target workspace"
    );
    let registry = fs::read_to_string(&registry_path).expect("registry file should read");
    assert!(registry.contains("\"providers\""));

    let draft_titles = snapshot
        .tasks
        .iter()
        .filter(|task| task.kind == CoordinationTaskKind::DraftDesignDoc)
        .map(|task| task.title.as_str())
        .collect::<Vec<_>>();
    assert!(!draft_titles.is_empty(), "run should seed provider draft tasks");
    assert!(draft_titles
        .iter()
        .all(|title| title.starts_with("Draft provider design proposal with ")));
    assert!(draft_titles
        .iter()
        .all(|title| !title.contains("canonical")));
}

#[test]
fn load_run_snapshot_returns_canceled_run_without_unblocking_tasks() {
    let fixture = CoordinationTestFixture::named("snapshot-canceled");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Canceled;
    let survey = fixture.make_task(
        &run,
        "survey",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let blocked = fixture.make_task(
        &run,
        "blocked",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Blocked plan",
        &["**"],
        &[],
        &["survey"],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[survey.clone(), blocked.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Canceled);
    assert_eq!(
        snapshot
            .tasks
            .iter()
            .find(|task| task.task_id == blocked.task_id)
            .expect("blocked task should remain")
            .status,
        CoordinationTaskStatus::Blocked
    );
}

#[test]
fn reconcile_run_state_preserves_explicit_canceled_status() {
    let fixture = CoordinationTestFixture::named("reconcile-canceled");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Canceled;
    let completed = fixture.make_task(
        &run,
        "completed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Completed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let canceled = fixture.make_task(
        &run,
        "canceled",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Canceled task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Canceled,
    );
    fixture.persist_run_with_tasks(&run, &[completed, canceled]);

    reconcile_run_state(&run.run_id).expect("reconcile should succeed");

    let reloaded = load_run(&run.run_id).expect("run should reload");
    assert_eq!(reloaded.status, CoordinationRunStatus::Canceled);
}

#[test]
fn load_run_snapshot_returns_freeform_council_run_without_task_derivation() {
    let fixture = CoordinationTestFixture::named("snapshot-council");
    let run = fixture.make_run(AutonomousWorkflowMode::FreeformCouncil);
    let blocked = fixture.make_task(
        &run,
        "blocked",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Blocked council task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[blocked.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(
        snapshot.run.workflow_mode,
        AutonomousWorkflowMode::FreeformCouncil
    );
    assert_eq!(
        snapshot.tasks[0].status,
        CoordinationTaskStatus::Blocked
    );
}

#[test]
fn load_run_snapshot_records_completion_gate_failures() {
    let fixture = CoordinationTestFixture::named("snapshot-gate-failure");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let completed = fixture.make_task(
        &run,
        "completed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Completed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[completed.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: completed.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "partial result".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "partial result".to_string(),
            details: "details".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Failed);
    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "run_completion_blocked"
            && event.message.contains("Completed task ended with outcome partial")
    }));
}

#[test]
fn load_run_snapshot_accepts_completed_task_summary_without_result_artifact() {
    let fixture = CoordinationTestFixture::named("snapshot-summary-fallback");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut completed = fixture.make_task(
        &run,
        "completed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Completed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    completed.artifact_summary = Some("completed via persisted task summary".to_string());
    fixture.persist_run_with_tasks(&run, &[completed.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
    let reloaded = load_task(&run.run_id, &completed.task_id).expect("task should reload");
    assert_eq!(reloaded.failure_kind, None);
    assert_eq!(reloaded.recovery_action, None);
}

#[test]
fn stop_run_cancels_active_tasks_clears_claims_and_preserves_retry_setting() {
    let fixture = CoordinationTestFixture::named("stop-run");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.auto_retry_failed_tasks = true;
    let mut running = fixture.make_task(
        &run,
        "running",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Running task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    running.claimed_by = Some(999_998);
    let queued = fixture.make_task(
        &run,
        "queued",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Queued task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    let completed = fixture.make_task(
        &run,
        "done",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Done task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[running.clone(), queued.clone(), completed.clone()]);
    assert!(
        create_claim_file(&make_claim_for_test(&run.run_id, &running.task_id, 999_998, 1234))
            .unwrap()
    );

    let snapshot = stop_run(&run.run_id, "user requested stop").expect("stop should succeed");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Canceled);
    assert!(snapshot.run.auto_retry_failed_tasks);
    let running_task = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == running.task_id)
        .expect("running task should exist");
    assert_eq!(running_task.status, CoordinationTaskStatus::Canceled);
    assert_eq!(
        running_task.artifact_summary.as_deref(),
        Some("user requested stop")
    );
    let queued_task = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == queued.task_id)
        .expect("queued task should exist");
    assert_eq!(queued_task.status, CoordinationTaskStatus::Canceled);
    let completed_task = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == completed.task_id)
        .expect("completed task should exist");
    assert_eq!(completed_task.status, CoordinationTaskStatus::Completed);
    assert!(!claim_path(&run.run_id, &running.task_id).exists());

    let restarted = restart_failed_tasks(&run.run_id).expect("restart should succeed");
    assert_eq!(restarted.run.status, CoordinationRunStatus::Active);
    assert!(restarted.run.auto_retry_failed_tasks);

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| event.event_type == "run_canceled"));
}

#[test]
fn update_run_max_agents_clamps_values_and_skips_noop_writes() {
    let fixture = CoordinationTestFixture::named("update-max-agents");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    write_json_atomic(&run_path(&run.run_id), &run).expect("run should persist");

    update_run_max_agents(&run.run_id, 99).expect("update should clamp");
    let clamped_high = load_run(&run.run_id).expect("run should reload");
    assert_eq!(clamped_high.max_agents, 8);

    update_run_max_agents(&run.run_id, 0).expect("update should clamp");
    let clamped_low = load_run(&run.run_id).expect("run should reload");
    assert_eq!(clamped_low.max_agents, 1);

    let unchanged_timestamp = clamped_low.updated_at;
    update_run_max_agents(&run.run_id, 1).expect("no-op update should succeed");
    let reloaded = load_run(&run.run_id).expect("run should reload");
    assert_eq!(reloaded.max_agents, 1);
    assert_eq!(reloaded.updated_at, unchanged_timestamp);
}

#[test]
fn restart_failed_tasks_requeues_missing_and_unparseable_gate_artifacts() {
    let fixture = CoordinationTestFixture::named("restart-gate-artifacts");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let mut missing = fixture.make_task(
        &run,
        "missing",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Missing artifact",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    missing.attempt_count = 2;
    missing.max_attempts = 2;
    let mut invalid = fixture.make_task(
        &run,
        "invalid",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Invalid artifact",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    invalid.attempt_count = 3;
    invalid.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[missing.clone(), invalid.clone()]);
    fs::create_dir_all(artifact_dir(&run.run_id, &invalid.task_id)).expect("artifact dir should exist");
    fs::write(
        artifact_result_path(&run.run_id, &invalid.task_id),
        "{ not valid json",
    )
    .expect("invalid artifact should write");

    let snapshot = restart_failed_tasks(&run.run_id).expect("restart should succeed");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    for task_id in [&missing.task_id, &invalid.task_id] {
        let task = snapshot
            .tasks
            .iter()
            .find(|task| &task.task_id == task_id)
            .expect("task should exist");
        assert_eq!(task.status, CoordinationTaskStatus::Queued);
        assert!(task.artifact_summary.is_none());
    }
    assert!(!artifact_result_path(&run.run_id, &invalid.task_id).exists());
}
