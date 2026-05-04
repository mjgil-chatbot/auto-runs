#[test]
fn complete_task_persists_materialized_files_for_changed_write_outputs() {
    let fixture = CoordinationTestFixture::named("persist-materialized-files");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "write-track",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write track artifact",
        &["docs/autonomous/design-doc.md"],
        &["docs/autonomous/prompts/track-2-cody-coleman.md"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(71);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
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

    let output_path = fixture
        .workspace
        .path()
        .join("docs/autonomous/prompts/track-2-cody-coleman.md");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::write(
        &output_path,
        "# Track 2\n\n## Stage 0 Validation\n\nPass.\n\n## Final Prompt\n\nPrompt.\n\n## Locked GTFA\n\n35\n\n## Step-by-Step Solution\n\n1. Step.\n\n## Numbered Required Sources\n\n1. Source.\n\n## Stump Reports\n\n- claude\n- gpt-5.4\n",
    )
    .unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "wrote track".to_string(),
        body: "Created docs/autonomous/prompts/track-2-cody-coleman.md".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "wrote track".to_string(),
            details: "Created docs/autonomous/prompts/track-2-cody-coleman.md".to_string(),
            changed_files: vec!["docs/autonomous/prompts/track-2-cody-coleman.md".to_string()],
            ..Default::default()
        },
    };

    complete_task(&run.run_id, &task.task_id, 71, artifact).unwrap();

    let raw =
        fs::read_to_string(artifact_materialized_files_path(&run.run_id, &task.task_id)).unwrap();
    let snapshots: Vec<MaterializedArtifactFile> = serde_json::from_str(&raw).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(
        snapshots[0].path,
        "docs/autonomous/prompts/track-2-cody-coleman.md"
    );
    assert!(snapshots[0].content.contains("# Track 2"));
}

#[test]
fn fail_task_marks_running_task_failed_and_clears_claim() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.auto_retry_failed_tasks = false;
    run.updated_at = Utc::now() - chrono::Duration::minutes(5);
    let mut task = fixture.make_task(
        &run,
        "failing-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Failing task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(81);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
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

    fail_task(
        &run.run_id,
        &task.task_id,
        81,
        "provider returned invalid report",
    )
    .unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.claimed_by, Some(81));
    assert_eq!(
        fs::read_to_string(artifact_notes_path(&run.run_id, &task.task_id)).unwrap(),
        "provider returned invalid report"
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let reloaded_run = load_run(&run.run_id).unwrap();
    assert_eq!(reloaded_run.status, CoordinationRunStatus::Failed);
    assert!(reloaded_run.updated_at > run.updated_at);

    let summary = query_runs_filtered(None, None, None, Some(200), None)
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.run_id == run.run_id)
        .expect("run summary should exist");
    assert_eq!(summary.status, "failed");
}

#[test]
fn fail_task_does_not_semantic_repair_input_rail_rejections() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut task = fixture.make_task(
        &run,
        "rail-failure",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split lightweight validation tasks",
        &["docs/design.md"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(82);
    task.attempt_count = 10;
    task.max_attempts = 10;
    task.task_category = Some(TaskCategory::Verify);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 82,
        launcher_pid: Some(82),
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

    let message = "input rail rejected task: required canonical design doc 'docs/autonomous/lightweight-implementation-doc-missing.md' is missing";
    fail_task(&run.run_id, &task.task_id, 82, message).unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.artifact_summary.as_deref(), Some(message));

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(!events.iter().any(|event| {
        event.event_type == "task_repair_scheduled"
            && event.task_id.as_deref() == Some(task.task_id.as_str())
    }));
}

#[test]
fn fail_task_does_not_auto_retry_input_rail_rejections() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut task = fixture.make_task(
        &run,
        "rail-retry",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split lightweight validation tasks",
        &["docs/design.md"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(83);
    task.attempt_count = 1;
    task.max_attempts = 3;
    task.task_category = Some(TaskCategory::Verify);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 83,
        launcher_pid: Some(83),
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

    let message = "input rail rejected task: required canonical design doc 'docs/autonomous/lightweight-implementation-doc-missing.md' is missing";
    fail_task(&run.run_id, &task.task_id, 83, message).unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(!events.iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "task_retry_scheduled" | "task_repair_scheduled"
        ) && event.task_id.as_deref() == Some(task.task_id.as_str())
    }));
}

#[test]
fn fail_task_does_not_semantic_repair_after_attempt_budget_exhausted() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let mut task = fixture.make_task(
        &run,
        "semantic-repair-exhausted",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Fix Service Worker Fetch Handler Precedence",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(84);
    task.attempt_count = 10;
    task.max_attempts = 10;
    task.task_category = Some(TaskCategory::DeepWrite);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 84,
        launcher_pid: Some(84),
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

    let message = "acceptance validation failed after semantic repair";
    fail_task(&run.run_id, &task.task_id, 84, message).unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.attempt_count, 10);
    assert_eq!(failed.max_attempts, 10);
    assert_eq!(failed.artifact_summary.as_deref(), Some(message));

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(!events.iter().any(|event| {
        event.event_type == "task_repair_scheduled"
            && event.task_id.as_deref() == Some(task.task_id.as_str())
    }));
}

#[test]
fn blocked_write_task_fails_and_waits_for_manual_restart() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "write-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(82);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 82,
        launcher_pid: Some(82),
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

    complete_task(
        &run.run_id,
        &task.task_id,
        82,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: Some("codex".to_string()),
            task_category: Some(TaskCategory::DeepWrite),
            fallback_from: None,
            summary: "blocked on missing dependency artifact".to_string(),
            body: "did not write file".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::Blocked,
                summary: "blocked on missing dependency artifact".to_string(),
                details: "did not write file".to_string(),
                blockers: vec!["missing dependency artifact".to_string()],
                ..Default::default()
            },
        },
    )
    .unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(
        failed.artifact_summary.as_deref(),
        Some("blocked on missing dependency artifact [blocked]")
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let reloaded = snapshot
        .tasks
        .iter()
        .find(|candidate| candidate.task_id == task.task_id)
        .unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Failed);
}

#[test]
fn restart_failed_tasks_requeues_completion_gate_failures() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
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
    completed.attempt_count = 3;
    completed.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[completed.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: completed.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "partial result".to_string(),
        body: "body".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "partial result".to_string(),
            details: "still missing one branch".to_string(),
            ..Default::default()
        },
    });

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == completed.task_id)
        .unwrap();

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(restarted.status, CoordinationTaskStatus::Queued);
    assert_eq!(restarted.max_attempts, 5);
    assert!(restarted.artifact_summary.is_none());
    assert!(!artifact_result_path(&run.run_id, &completed.task_id).exists());
}

#[test]
fn restart_failed_tasks_grants_two_additional_attempts_to_failed_tasks() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let mut failed = fixture.make_task(
        &run,
        "failed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Failed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 4;
    failed.max_attempts = 4;
    failed.artifact_summary = Some("previous failure".to_string());
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .unwrap();

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(restarted.status, CoordinationTaskStatus::Queued);
    assert_eq!(restarted.attempt_count, 4);
    assert_eq!(restarted.max_attempts, 6);
    assert!(restarted.artifact_summary.is_none());
}

#[test]
fn load_run_snapshot_auto_fixes_failed_lightweight_run_once() {
    let fixture = CoordinationTestFixture::named("auto-fix-lightweight-run");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.auto_retry_failed_tasks = true;
    run.notes =
        lightweight_notes_with_repo_configuration(&run.notes, LightweightRepoConfiguration::Rust);
    let mut failed = fixture.make_task(
        &run,
        "write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write fix",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 3;
    failed.max_attempts = 3;
    failed.artifact_summary = Some("worker failed".to_string());
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let repair = snapshot
        .tasks
        .iter()
        .find(|task| task.title == "Repair repository after failed run")
        .expect("repair task should be generated");
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("restarted task should exist");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(repair.status, CoordinationTaskStatus::Queued);
    assert_eq!(repair.execution_generation, 2);
    assert!(repair.prompt.contains("cargo build"));
    assert!(repair.prompt.contains("cargo test"));
    assert_eq!(restarted.status, CoordinationTaskStatus::Blocked);
    assert_eq!(restarted.dependencies, vec![repair.task_id.clone()]);
    assert_eq!(restarted.execution_generation, 2);
    assert_eq!(restarted.max_attempts, 5);
    assert!(snapshot
        .run
        .notes
        .contains("auto_fix_failed_run_restarts_used=1"));
    assert!(fixture.workspace.path().join(".git").is_dir());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "run_auto_fix_restart_scheduled"
            && event.message.contains("restart 1/1")
    }));

    let mut failed_again = load_task(&run.run_id, &failed.task_id).unwrap();
    failed_again.status = CoordinationTaskStatus::Failed;
    failed_again.attempt_count = failed_again.max_attempts;
    failed_again.artifact_summary = Some("failed again".to_string());
    failed_again.updated_at = Utc::now();
    write_json_atomic(
        &task_path(&run.run_id, &failed_again.task_id),
        &failed_again,
    )
    .unwrap();
    let mut rerun = load_run(&run.run_id).unwrap();
    rerun.status = CoordinationRunStatus::Failed;
    rerun.updated_at = Utc::now();
    write_json_atomic(&run_path(&run.run_id), &rerun).unwrap();

    let second_snapshot = load_run_snapshot(&run.run_id).unwrap();
    let second_failed = second_snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("failed task should still exist");
    let repair_count = second_snapshot
        .tasks
        .iter()
        .filter(|task| task.title == "Repair repository after failed run")
        .count();
    assert_eq!(second_snapshot.run.status, CoordinationRunStatus::Failed);
    assert_eq!(second_failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(auto_fix_failed_run_restarts_used(&second_snapshot.run), 1);
    assert_eq!(repair_count, 1);
}

#[test]
fn fail_task_immediately_auto_fixes_failed_lightweight_run() {
    let fixture = CoordinationTestFixture::named("fail-task-auto-fix-lightweight-run");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.auto_retry_failed_tasks = true;
    run.notes =
        lightweight_notes_with_repo_configuration(&run.notes, LightweightRepoConfiguration::Zig);
    let mut failed = fixture.make_task(
        &run,
        "write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Write fix",
        &["src/lib.zig"],
        &["src/lib.zig"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 5;
    failed.max_attempts = 5;
    failed.artifact_summary = Some("minimax task timed out after 35 minutes.".to_string());
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let now = Utc::now();
    sync_run_summary_after_task_update(&run.run_id, now).unwrap();

    let persisted_run = load_run(&run.run_id).unwrap();
    let persisted_tasks = load_tasks(&run.run_id).unwrap();
    let restarted = persisted_tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("restarted task should exist");

    assert_eq!(persisted_run.status, CoordinationRunStatus::Active);
    assert!(matches!(
        restarted.status,
        CoordinationTaskStatus::Queued | CoordinationTaskStatus::Blocked
    ));
    assert_eq!(restarted.max_attempts, 7);
    assert!(persisted_run
        .notes
        .contains("auto_fix_failed_run_restarts_used=1"));
    assert!(
        persisted_tasks
            .iter()
            .all(|task| task.status != CoordinationTaskStatus::Failed)
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "run_auto_fix_restart_scheduled"
            && event.message.contains("restart 1/1")
    }));
}

#[test]
fn auto_fix_restart_reuses_existing_repair_task() {
    let fixture = CoordinationTestFixture::named("auto-fix-reuses-repair-task");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.auto_retry_failed_tasks = true;
    run.notes =
        lightweight_notes_with_repo_configuration(&run.notes, LightweightRepoConfiguration::Rust);
    let mut failed = fixture.make_task(
        &run,
        "write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write fix",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 3;
    failed.max_attempts = 3;
    failed.artifact_summary = Some("worker failed".to_string());
    let repair = fixture.make_task(
        &run,
        "repair",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Repair repository after failed run",
        &["**"],
        &["**"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[failed.clone(), repair.clone()]);

    let snapshot =
        restart_failed_tasks_with_mode(&run.run_id, RestartMode::AutoSelfHeal).unwrap();
    let repair_tasks = snapshot
        .tasks
        .iter()
        .filter(|task| task.title == "Repair repository after failed run")
        .collect::<Vec<_>>();
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("restarted task should exist");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(repair_tasks.len(), 1);
    assert_eq!(repair_tasks[0].task_id, repair.task_id);
    assert_eq!(restarted.status, CoordinationTaskStatus::Blocked);
    assert_eq!(restarted.dependencies, vec![repair.task_id.clone()]);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(!events.iter().any(|event| {
        event.event_type == "task_generated"
            && event
                .message
                .contains("Generated post-failure repair task 'Repair repository after failed run'.")
    }));
}

#[test]
fn manual_restart_lightweight_run_keeps_direct_retry_without_repair_task() {
    let fixture = CoordinationTestFixture::named("manual-lightweight-restart");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.status = CoordinationRunStatus::Failed;
    let mut failed = fixture.make_task(
        &run,
        "write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write fix",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 3;
    failed.max_attempts = 3;
    failed.artifact_summary = Some("worker failed".to_string());
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("restarted task should exist");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(restarted.status, CoordinationTaskStatus::Queued);
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.title != "Repair repository after failed run"));
}

#[test]
fn restart_failed_tasks_resets_completed_dependents_of_restarted_tasks() {
    let fixture = CoordinationTestFixture::named("restart-dependency-graph");
    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    run.status = CoordinationRunStatus::Failed;
    let mut draft = fixture.make_task(
        &run,
        "draft",
        CoordinationTaskKind::DraftDesignDoc,
        Some("gemini"),
        "Draft provider design proposal with gemini-3-flash-preview",
        &["instructions/**"],
        &[],
        &[],
        CoordinationTaskStatus::Failed,
    );
    draft.attempt_count = 2;
    draft.max_attempts = 5;
    draft.artifact_summary = Some("provider failed".to_string());
    let mut converge = fixture.make_task(
        &run,
        "converge",
        CoordinationTaskKind::ConvergeDesignDoc,
        Some("codex"),
        "Converge design doc",
        &["artifacts/**"],
        &[],
        &["draft"],
        CoordinationTaskStatus::Completed,
    );
    converge.artifact_summary = Some("converged".to_string());
    fixture.persist_run_with_tasks(&run, &[draft.clone(), converge.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: converge.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Design),
        fallback_from: None,
        summary: "converged".to_string(),
        body: "design doc".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "converged".to_string(),
            details: "design doc".to_string(),
            ..Default::default()
        },
    });

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted_draft = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == draft.task_id)
        .expect("draft task should exist");
    let reset_converge = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == converge.task_id)
        .expect("converge task should exist");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(restarted_draft.status, CoordinationTaskStatus::Queued);
    assert_eq!(reset_converge.status, CoordinationTaskStatus::Blocked);
    assert!(reset_converge.artifact_summary.is_none());
    assert!(!artifact_result_path(&run.run_id, &converge.task_id).exists());
    assert!(!artifact_notes_path(&run.run_id, &converge.task_id).exists());
}

#[test]
fn restart_failed_tasks_rebases_completed_dependencies_without_rerunning_them() {
    let fixture = CoordinationTestFixture::named("restart-upstream-generation");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let now = Utc::now();
    let mut dependency = fixture.make_task(
        &run,
        "dependency",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Inspect workspace",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    dependency.execution_generation = 1;
    dependency.artifact_summary = Some("workspace inspected".to_string());
    let mut failed = fixture.make_task(
        &run,
        "failed-write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Fix the bug",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &["dependency"],
        CoordinationTaskStatus::Failed,
    );
    failed.execution_generation = 1;
    failed.attempt_count = 2;
    failed.max_attempts = 3;
    failed.artifact_summary = Some("stale blocked result".to_string());
    fixture.persist_run_with_tasks(&run, &[dependency.clone(), failed.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: dependency.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "workspace inspected".to_string(),
        body: "stale dependency artifact".to_string(),
        completed_at: now,
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "workspace inspected".to_string(),
            details: "stale dependency artifact".to_string(),
            ..Default::default()
        },
    });

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted_dependency = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == dependency.task_id)
        .expect("dependency should exist");
    let restarted_failed = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == failed.task_id)
        .expect("failed task should exist");

    assert_eq!(restarted_dependency.execution_generation, 2);
    assert_eq!(restarted_failed.execution_generation, 2);
    assert_eq!(
        restarted_dependency.status,
        CoordinationTaskStatus::Completed
    );
    assert_eq!(restarted_failed.status, CoordinationTaskStatus::Queued);
    assert!(artifact_notes_path(&run.run_id, &dependency.task_id).exists());
    assert!(!archived_artifact_notes_path(&run.run_id, &dependency.task_id, 1).exists());
}

#[test]
fn mark_task_failed_for_restart_flips_completed_task_and_run_to_failed() {
    let fixture = CoordinationTestFixture::named("mark-run-failed");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Completed;
    let completed = fixture.make_task(
        &run,
        "done",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Done task",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[completed]);

    let snapshot =
        mark_task_failed_for_restart(&run.run_id, "done", "User requested restart").unwrap();

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Failed);
    let failed_task = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == "done")
        .expect("task should still exist");
    assert_eq!(failed_task.status, CoordinationTaskStatus::Failed);
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_marked_failed" && event.message.contains("User requested restart")
    }));
}

#[test]
fn set_task_assigned_provider_updates_task_and_records_event() {
    let fixture = CoordinationTestFixture::named("reassign-provider");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let failed = fixture.make_task(
        &run,
        "failed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Failed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    fixture.persist_run_with_tasks(&run, &[failed]);

    set_task_assigned_provider(&run.run_id, "failed", "Claude").unwrap();

    let updated = load_task(&run.run_id, "failed").unwrap();
    assert_eq!(updated.assigned_provider.as_deref(), Some("claude"));
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_provider_reassigned"
            && event.message.contains("Updated assigned provider")
    }));
}

#[test]
fn restart_failed_tasks_keeps_reassigned_provider() {
    let fixture = CoordinationTestFixture::named("restart-reassigned-provider");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let mut failed = fixture.make_task(
        &run,
        "failed",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Failed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 2;
    failed.max_attempts = 2;
    failed.artifact_summary = Some("previous failure".to_string());
    fixture.persist_run_with_tasks(&run, &[failed]);

    set_task_assigned_provider(&run.run_id, "failed", "claude").unwrap();
    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    let restarted = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == "failed")
        .expect("task should exist");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert_eq!(restarted.status, CoordinationTaskStatus::Queued);
    assert_eq!(restarted.assigned_provider.as_deref(), Some("claude"));
}

#[test]
fn load_events_with_limit_returns_newest_events_in_file_order() {
    let fixture = CoordinationTestFixture::named("recent-events-tail");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    fixture.persist_run_with_tasks(&run, &[]);

    append_event(&run.run_id, "event_1", None, "first").unwrap();
    append_event(&run.run_id, "event_2", None, "second").unwrap();
    append_event(&run.run_id, "event_3", None, "third").unwrap();

    let events = load_events(&run.run_id, Some(2), None).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event_type, "event_2");
    assert_eq!(events[0].message, "second");
    assert_eq!(events[1].event_type, "event_3");
    assert_eq!(events[1].message, "third");
}

#[test]
fn restart_failed_tasks_uses_persisted_glm_cooldown_state() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let mut failed = fixture.make_task(
        &run,
        "failed",
        CoordinationTaskKind::DraftDesignDoc,
        Some("glm"),
        "Draft provider design proposal with GLM 5.1",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 4;
    failed.max_attempts = 4;
    failed.task_category = Some(TaskCategory::Design);
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let mut global_cooldowns = BTreeMap::new();
    global_cooldowns.insert(
        "glm".to_string(),
        ProviderCooldown {
            unavailable_until: Utc::now() + chrono::Duration::hours(3),
            reason: "GLM API quota exhausted.".to_string(),
        },
    );
    write_global_provider_cooldowns(&global_cooldowns).unwrap();

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();

    assert!(snapshot.run.provider_cooldowns.contains_key("glm"));
    let reassigned = resolve_provider_for_task_claim_with_category(
        &snapshot.run,
        Some("glm"),
        "glm",
        failed.task_category.as_ref(),
        Utc::now(),
    );
    assert!(reassigned.is_some());
    assert_ne!(reassigned.as_deref(), Some("glm"));
}

#[test]
fn restart_failed_tasks_backfills_glm_cooldown_into_shared_state_from_failure_summary() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Failed;
    let mut failed = fixture.make_task(
        &run,
        "failed",
        CoordinationTaskKind::DraftDesignDoc,
        Some("glm"),
        "Draft provider design proposal with GLM 5.1",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Failed,
    );
    failed.attempt_count = 4;
    failed.max_attempts = 4;
    failed.task_category = Some(TaskCategory::Design);
    failed.artifact_summary = Some("Claude command failed: API Error: 429 {\"error\":{\"code\":\"1308\",\"message\":\"已达到 5 小时的使用上限。您的限额将在 2099-12-31 23:59:59 重置。\"},\"request_id\":\"20260408015107f96597ccc364431a\"}".to_string());
    fixture.persist_run_with_tasks(&run, &[failed.clone()]);

    let snapshot = restart_failed_tasks(&run.run_id).unwrap();
    assert!(snapshot.run.provider_cooldowns.contains_key("glm"));

    let global = load_global_provider_cooldowns().unwrap();
    assert!(global.contains_key("glm"));

    let reassigned = resolve_provider_for_task_claim_with_category(
        &snapshot.run,
        Some("glm"),
        "glm",
        failed.task_category.as_ref(),
        Utc::now(),
    );
    assert!(reassigned.is_some());
    assert_ne!(reassigned.as_deref(), Some("glm"));
}

#[test]
fn completion_and_restart_tests_use_tempdir_isolation() {
    let fixture = CoordinationTestFixture::new();
    let shared_root = config::shared_infra_dir();

    assert!(shared_root.starts_with(Path::new(std::env::var("HOME").unwrap().as_str())));
    assert!(fixture.workspace.path().exists());
}
