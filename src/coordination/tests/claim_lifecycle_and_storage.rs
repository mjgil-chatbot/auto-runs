#[test]
fn record_task_execution_started_updates_claim_and_appends_event() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "worker-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Worker task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(73);
    let now = Utc::now();
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 73,
        launcher_pid: Some(41),
        lifecycle_phase: ClaimLifecyclePhase::WorkerAdopted,
        worker_pid: Some(73),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: None,
        worker_exited_at: Some(now),
        worker_exit_detail: Some("previous exit".to_string()),
        last_heartbeat_at: now,
    })
    .unwrap());

    record_task_execution_started(&run.run_id, &task.task_id, 73).unwrap();

    let claim = load_claim(&run.run_id, &task.task_id).unwrap();
    assert_eq!(
        claim.lifecycle_phase,
        ClaimLifecyclePhase::TaskExecutionStarted
    );
    assert_eq!(claim.worker_pid, Some(73));
    assert_eq!(claim.worker_adopted_at, Some(now));
    assert!(claim.execution_started_at.is_some());
    assert_eq!(claim.worker_exited_at, None);
    assert_eq!(claim.worker_exit_detail, None);

    let events = load_events(&run.run_id, None, None).unwrap();
    let execution_event = events
        .iter()
        .find(|event| event.event_type == "task_execution_started")
        .expect("execution start event should be recorded");
    assert_eq!(
        execution_event.task_id.as_deref(),
        Some(task.task_id.as_str())
    );
    assert_eq!(execution_event.provider.as_deref(), Some("codex"));
    assert!(execution_event
        .message
        .contains("Worker PID 73 began task execution"));
    assert!(execution_event
        .message
        .contains("phase=task_execution_started"));
}

#[test]
fn count_active_claims_skips_malformed_files_and_counts_only_live_claims() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let live = fixture.make_task(
        &run,
        "count-live",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Live claim",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    let expired = fixture.make_task(
        &run,
        "count-expired",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Expired claim",
        &["src/main.rs"],
        &["src/main.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    fixture.persist_run_with_tasks(&run, &[live.clone(), expired.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: live.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 81,
        launcher_pid: Some(81),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    })
    .unwrap());
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: expired.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 82,
        launcher_pid: Some(82),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1),
    })
    .unwrap());
    fs::write(claims_dir(&run.run_id).join("bad.json"), "{\"broken\":").unwrap();

    assert_eq!(count_active_claims(&run.run_id).unwrap(), 1);
}

#[test]
fn storage_round_trips_run_task_event_artifact_and_review_metadata() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "review-task",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review state",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let metadata = code_review::ReviewTaskMetadata {
        scope_description: "Current workspace".to_string(),
        dimensions: vec!["architecture".to_string()],
        chunk: vec![code_review::ReviewChunkTarget {
            path: "src/lib.rs".to_string(),
            start_line: 1,
            end_line: 8,
        }],
        automated_checks: Vec::new(),
        profile_path: code_review::REVIEW_PROFILE_PATH.to_string(),
        profile_warnings: vec!["profile warning".to_string()],
        fix_mode: Some(ReviewFixMode::ReportOnly),
        output_path: Some("docs/review.md".to_string()),
    };
    write_review_task_metadata(&run.run_id, &task.task_id, &metadata).unwrap();
    append_event(&run.run_id, "review_completed", Some(&task.task_id), "done").unwrap();

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "reviewed".to_string(),
        body: "# Review artifact".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "reviewed".to_string(),
            details: "checked".to_string(),
            ..Default::default()
        },
    });

    let loaded_run = load_run(&run.run_id).unwrap();
    let loaded_task = load_task(&run.run_id, &task.task_id).unwrap();
    let loaded_tasks = load_tasks(&run.run_id).unwrap();
    let loaded_events = load_events(&run.run_id, None, None).unwrap();
    let loaded_metadata = load_review_task_metadata(&run.run_id, &task.task_id).unwrap();
    let dependency_notes = load_dependency_artifacts(&run.run_id, &[task.task_id.clone()]).unwrap();

    assert_eq!(loaded_run.run_id, run.run_id);
    assert_eq!(loaded_task.title, "Review state");
    assert_eq!(loaded_tasks.len(), 1);
    assert_eq!(loaded_events[0].event_type, "review_completed");
    assert_eq!(
        loaded_metadata.output_path.as_deref(),
        Some("docs/review.md")
    );
    assert_eq!(dependency_notes, vec!["# Review artifact".to_string()]);
}

#[test]
fn load_tasks_ignores_atomic_write_temp_files() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let task = fixture.make_task(
        &run,
        "real-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Real task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let temp_path = tasks_dir(&run.run_id).join(format!(".{}.json.tmp-12345", task.task_id));
    fs::write(&temp_path, "").unwrap();

    let loaded_tasks = load_tasks(&run.run_id).unwrap();
    assert_eq!(loaded_tasks.len(), 1);
    assert_eq!(loaded_tasks[0].task_id, task.task_id);
}

#[test]
fn reap_stale_claim_files_requeues_running_task_and_removes_claim_file() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let task = fixture.make_task(
        &run,
        "stale-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Stale task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    // Create a claim file with a stale heartbeat (older than TASK_HEARTBEAT_STALE_SECS).
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 73,
        launcher_pid: Some(41),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(73),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 30),
    })
    .unwrap());

    let reaped = reap_stale_claim_files(&run.run_id).unwrap();
    assert_eq!(reaped, 1);

    let loaded_task = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(loaded_task.status, CoordinationTaskStatus::Queued);
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn reap_stale_claim_files_requeues_task_when_attempts_are_exhausted() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let mut task = fixture.make_task(
        &run,
        "exhausted-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Exhausted task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 3;
    task.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    // Create a claim file with a stale heartbeat.
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 73,
        launcher_pid: Some(41),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(73),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 30),
    })
    .unwrap());

    let reaped = reap_stale_claim_files(&run.run_id).unwrap();
    assert_eq!(reaped, 1);

    let loaded_task = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(loaded_task.status, CoordinationTaskStatus::Queued);
    assert_eq!(loaded_task.attempt_count, 2);
    assert_eq!(loaded_task.failure_kind, Some(TaskFailureKind::ClaimLost));
    assert_eq!(
        loaded_task.recovery_action,
        Some(TaskRecoveryAction::ReconcileState)
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn reap_stale_claim_files_ignores_non_running_tasks_and_removes_orphaned_claims() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();

    // A task that is Completed (not Running) — should not be touched.
    let completed_task = fixture.make_task(
        &run,
        "completed-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Completed task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );

    // A task that is Queued — should not be touched.
    let queued_task = fixture.make_task(
        &run,
        "queued-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Queued task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    fixture.persist_run_with_tasks(&run, &[completed_task.clone(), queued_task.clone()]);

    // Create claim files for both — orphaned (task not Running).
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: completed_task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 73,
        launcher_pid: Some(41),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(73),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 30),
    })
    .unwrap());
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: queued_task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 73,
        launcher_pid: Some(41),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 30),
    })
    .unwrap());

    let reaped = reap_stale_claim_files(&run.run_id).unwrap();
    // Neither task is Running, so no reaping should occur.
    assert_eq!(reaped, 0);

    // Both tasks keep their original status.
    let loaded_completed = load_task(&run.run_id, &completed_task.task_id).unwrap();
    let loaded_queued = load_task(&run.run_id, &queued_task.task_id).unwrap();
    assert_eq!(loaded_completed.status, CoordinationTaskStatus::Completed);
    assert_eq!(loaded_queued.status, CoordinationTaskStatus::Queued);
    // Claim files for non-Running tasks should still be removed (orphaned cleanup).
    assert!(!claim_path(&run.run_id, &completed_task.task_id).exists());
    assert!(!claim_path(&run.run_id, &queued_task.task_id).exists());
}
