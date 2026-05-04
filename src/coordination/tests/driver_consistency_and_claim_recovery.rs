#[test]
fn claim_next_runnable_task_appends_driver_context_when_status_matches() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "claimable",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Claimable task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task]);

    write_json_atomic(
        &coordination_root_dir().join("background-driver.json"),
        &crate::driver::DriverStatus {
            pid: 41,
            heartbeat_at: Utc::now(),
            poll_interval_ms: 500,
            started_at: Some(Utc::now()),
            executable_path: "/tmp/auto-runs".to_string(),
            executable_deleted: false,
            stop_reason: None,
        },
    )
    .unwrap();

    claim_next_runnable_task(&run.run_id, 41, "codex")
        .unwrap()
        .expect("task should claim");

    let events = load_events(&run.run_id, None, None).unwrap();
    let claimed = events
        .iter()
        .find(|event| event.event_type == "task_claimed")
        .expect("task_claimed event should exist");
    assert!(claimed.message.contains("driver pid=41"));
    assert!(claimed.message.contains("deleted=false"));
}

#[test]
fn load_run_snapshot_requeues_running_task_with_missing_claim() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "missing-claim",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Missing claim task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(77);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let repaired = snapshot
        .tasks
        .iter()
        .find(|candidate| candidate.task_id == task.task_id)
        .expect("task should still exist");
    assert_eq!(repaired.status, CoordinationTaskStatus::Queued);
    assert_eq!(repaired.claimed_by, None);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claim_missing"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_requeued"));
}

#[test]
fn load_run_snapshot_fails_exhausted_running_task_with_missing_claim() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "missing-claim-exhausted",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Missing claim exhausted task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 3;
    task.max_attempts = 3;
    task.claimed_by = Some(88);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let repaired = snapshot
        .tasks
        .iter()
        .find(|candidate| candidate.task_id == task.task_id)
        .expect("task should still exist");
    assert_eq!(repaired.status, CoordinationTaskStatus::Queued);
    assert!(repaired.artifact_summary.is_none());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claim_missing"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_requeued"));
}

#[test]
fn stale_missing_claim_repair_skips_task_that_is_no_longer_active() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "missing-claim-stale",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Missing claim stale task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(55);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let stale_snapshot = load_task(&run.run_id, &task.task_id).unwrap();
    let mut completed = stale_snapshot.clone();
    completed.status = CoordinationTaskStatus::Completed;
    completed.claimed_by = None;
    completed.updated_at = Utc::now();
    write_json_atomic(&task_path(&run.run_id, &task.task_id), &completed).unwrap();

    assert!(!reconcile_missing_claim_for_active_task_from_snapshot(
        &run.run_id,
        &stale_snapshot,
        Utc::now(),
    )
    .unwrap());

    let events = load_events(&run.run_id, None, None).unwrap_or_default();
    assert!(!events
        .iter()
        .any(|event| event.event_type == "task_claim_missing"));
    assert!(!events
        .iter()
        .any(|event| event.event_type == "task_requeued"));

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Completed);
}

#[test]
fn load_run_snapshot_removes_orphaned_claim_file_for_terminal_task() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "orphan-claim",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Orphan claim task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 1337,
        launcher_pid: Some(1337),
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

    let _snapshot = load_run_snapshot(&run.run_id).unwrap();

    assert!(!claim_path(&run.run_id, &task.task_id).exists());
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claim_orphaned"));
}

#[test]
fn load_run_snapshot_ignores_claim_only_drift_for_active_task() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let mut task = fixture.make_task(
        &run,
        "claim-drift",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Claim drift task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(100);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 100,
        launcher_pid: Some(100),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(100),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    })
    .unwrap());

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let repaired = snapshot
        .tasks
        .iter()
        .find(|candidate| candidate.task_id == task.task_id)
        .expect("task should still exist");
    assert_eq!(repaired.claimed_by, Some(100));

    let events = load_events(&run.run_id, None, None).unwrap_or_default();
    assert!(!events
        .iter()
        .any(|event| event.event_type == "task_claim_reconciled"));
}

#[test]
fn load_run_snapshot_repairs_claim_owner_drift_for_active_task() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let mut task = fixture.make_task(
        &run,
        "owner-drift",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Owner drift task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(100);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 200,
        launcher_pid: Some(100),
        lifecycle_phase: ClaimLifecyclePhase::WorkerAdopted,
        worker_pid: Some(200),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    })
    .unwrap());

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let repaired = snapshot
        .tasks
        .iter()
        .find(|candidate| candidate.task_id == task.task_id)
        .expect("task should still exist");
    assert_eq!(repaired.claimed_by, Some(200));

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claim_reconciled"));
}

#[test]
fn drive_run_for_instance_recovers_missing_claim_before_continuing() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "survey",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(91);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(drive_run_for_instance(&run.run_id, 71, "codex").unwrap());

    let completed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claim_missing"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_completed"));
}

#[test]
fn drive_active_runs_once_continues_after_broken_freeform_council() {
    let fixture = CoordinationTestFixture::new();

    let mut broken_run = fixture.make_run(AutonomousWorkflowMode::FreeformCouncil);
    broken_run.created_at = Utc::now() + chrono::Duration::seconds(5);
    broken_run.updated_at = broken_run.created_at;
    write_json_atomic(&run_path(&broken_run.run_id), &broken_run).unwrap();
    std::fs::write(council_json_path(&broken_run.run_id), "{not valid json").unwrap();

    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "survey-after-broken-council",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(crate::driver::drive_active_runs_once(71).unwrap());

    let completed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);
}

#[test]
fn drive_active_runs_once_does_not_drop_older_live_runs_behind_terminal_run_limit() {
    let fixture = CoordinationTestFixture::new();

    let mut older_live_run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    older_live_run.created_at = Utc::now() - chrono::Duration::days(7);
    older_live_run.updated_at = older_live_run.created_at;
    let live_task = fixture.make_task(
        &older_live_run,
        "survey-older-live-run",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&older_live_run, &[live_task.clone()]);

    for idx in 0..205 {
        let mut completed_run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
        completed_run.run_id = format!("completed-run-{idx:03}");
        completed_run.created_at = Utc::now() + chrono::Duration::seconds(idx.into());
        completed_run.updated_at = completed_run.created_at;
        completed_run.status = CoordinationRunStatus::Completed;
        fixture.persist_run_with_tasks(&completed_run, &[]);
    }

    assert!(crate::driver::drive_active_runs_once(71).unwrap());

    let completed = load_task(&older_live_run.run_id, &live_task.task_id).unwrap();
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);
}

// ── 096d1ba: re-repair loop when worker heartbeat goes stale ────────────────

/// Regression test: when expire_stale_claims removes a stale claim and requeues
/// a task, reconcile_tasks_missing_claims must NOT re-repair the same task in
/// the same iteration. The fix skips tasks with failure_kind=ClaimLost.
#[test]
fn expire_stale_claims_then_reconcile_missing_claims_does_not_rerepair() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();

    // Task in Running state with a dead worker PID.
    let mut task = fixture.make_task(
        &run,
        "stale-heartbeat",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Stale heartbeat task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.claimed_by = Some(9999); // PID that won't exist.
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    // Create a claim file with a stale heartbeat.
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 9999,
        launcher_pid: Some(9999),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(9999),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 30),
    })
    .unwrap());

    // Simulate what drive_run_for_instance does: call expire_stale_claims,
    // then reconcile_tasks_missing_claims. Before the fix, the second call would
    // re-repair the same task, causing a re-repair loop.
    expire_stale_claims(&run.run_id).unwrap();
    let second_pass_changed = reconcile_tasks_missing_claims(&run.run_id).unwrap();

    // The second pass must not change anything — expire_stale_claims already
    // handled the task.
    assert!(!second_pass_changed, "reconcile_tasks_missing_claims should not re-repair a task already handled by expire_stale_claims");

    let loaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(loaded.status, CoordinationTaskStatus::Queued);
    assert_eq!(loaded.failure_kind, Some(TaskFailureKind::ClaimLost));
}

/// Regression test: ensure reconcile_tasks_missing_claims skips repair for a
/// task whose failure_kind is already ClaimLost, preventing repeated re-queues.
#[test]
fn reconcile_tasks_missing_claims_skips_task_with_existing_claimlost_failure_kind() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);

    // Task already in Queued state with failure_kind=ClaimLost (simulating a
    // prior expire_stale_claims pass that hasn't been persisted properly yet).
    let mut task = fixture.make_task(
        &run,
        "already-repaired",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Already repaired task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued, // Prior repair is already durable.
    );
    task.attempt_count = 1;
    task.claimed_by = None;
    task.failure_kind = Some(TaskFailureKind::ClaimLost); // ...but already marked ClaimLost.
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    // No claim file exists (expire_stale_claims removed it).
    // reconcile_tasks_missing_claims should NOT repair again.
    let changed = reconcile_tasks_missing_claims(&run.run_id).unwrap();

    assert!(
        !changed,
        "reconcile_tasks_missing_claims must skip a task already marked ClaimLost"
    );

    let loaded = load_task(&run.run_id, &task.task_id).unwrap();
    // Status should remain Queued (from expire_stale_claims), not be re-set.
    assert_eq!(loaded.status, CoordinationTaskStatus::Queued);
}
