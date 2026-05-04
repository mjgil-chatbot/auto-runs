#[test]
fn claim_next_runnable_task_marks_task_running_and_writes_claim() {
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
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let claimed = claim_next_runnable_task(&run.run_id, 41, "codex")
        .unwrap()
        .expect("task should claim");

    assert_eq!(claimed.task_id, task.task_id);
    assert_eq!(claimed.status, CoordinationTaskStatus::Running);
    assert_eq!(claimed.claimed_by, Some(41));
    assert_eq!(claimed.assigned_provider.as_deref(), Some("codex"));

    let persisted = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Running);
    assert_eq!(persisted.claimed_by, Some(41));
    assert_eq!(persisted.attempt_count, 1);

    let claim = load_claim(&run.run_id, &task.task_id).unwrap();
    assert_eq!(claim.instance_pid, 41);
    assert_eq!(claim.provider, "codex");
    assert_eq!(count_active_claims(&run.run_id).unwrap(), 1);
}

#[test]
fn claim_next_runnable_task_reblocks_stale_queued_tasks_with_unmet_dependencies() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let draft = fixture.make_task(
        &run,
        "draft",
        CoordinationTaskKind::DraftDesignDoc,
        Some("qwen"),
        "Draft provider design proposal with Qwen 3.6 Free",
        &["instructions/**", "plan.txt"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    let materialize = fixture.make_task(
        &run,
        "follow-up",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("qwen"),
        "Follow-up write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &["draft"],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[draft.clone(), materialize.clone()]);
    persist_live_test_claim(&draft);

    let claimed = claim_next_runnable_task(&run.run_id, 41, "qwen").unwrap();

    assert!(claimed.is_none());
    let persisted = load_task(&run.run_id, &materialize.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Blocked);
    assert!(persisted.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &materialize.task_id).exists());
}

#[test]
fn validate_task_artifact_allows_unclaimed_out_of_scope_change() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let current = fixture.make_task(
        &run,
        "current",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Current task",
        &["src/debug.rs", "src/data.rs"],
        &[
            "src/debug.rs",
            "src/data.rs",
            "src/worker_ext/background.rs",
        ],
        &[],
        CoordinationTaskStatus::Running,
    );
    let unrelated = fixture.make_task(
        &run,
        "unrelated",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Unrelated task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[current.clone(), unrelated]);

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: current.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "done".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "done".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            ..Default::default()
        },
    };

    let errors = validate_task_artifact(&run.run_id, &current, &artifact).unwrap();
    assert!(
        errors.is_empty(),
        "unexpected validation errors: {:?}",
        errors
    );
}

#[test]
fn validate_task_artifact_rejects_out_of_scope_change_when_other_task_claims_scope() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let current = fixture.make_task(
        &run,
        "current",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Current task",
        &["src/debug.rs", "src/data.rs"],
        &[
            "src/debug.rs",
            "src/data.rs",
            "src/worker_ext/background.rs",
        ],
        &[],
        CoordinationTaskStatus::Running,
    );
    let owner = fixture.make_task(
        &run,
        "owner",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Owning task",
        &["src/data/tests.rs"],
        &["src/data/tests.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    let mut owner = owner;
    owner.claimed_by = Some(52);

    fixture.persist_run_with_tasks(&run, &[current.clone(), owner.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: owner.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 52,
        launcher_pid: Some(52),
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

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: current.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "done".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "done".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            ..Default::default()
        },
    };

    let errors = validate_task_artifact(&run.run_id, &current, &artifact).unwrap();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("src/data/tests.rs"));
}

#[test]
fn validate_task_artifact_allows_out_of_scope_change_after_other_task_completed() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let current = fixture.make_task(
        &run,
        "current",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Current task",
        &["src/debug.rs", "src/data.rs"],
        &[
            "src/debug.rs",
            "src/data.rs",
            "src/worker_ext/background.rs",
        ],
        &[],
        CoordinationTaskStatus::Running,
    );
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
    fixture.persist_run_with_tasks(&run, &[current.clone(), completed.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: completed.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "done".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "done".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            ..Default::default()
        },
    });

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: current.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "done".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "done".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            ..Default::default()
        },
    };

    let errors = validate_task_artifact(&run.run_id, &current, &artifact).unwrap();
    assert!(
        errors.is_empty(),
        "completed work should not block later retries: {:?}",
        errors
    );
}

#[test]
fn claim_next_runnable_task_expands_retry_scope_after_scope_holder_finishes() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut retry_task = fixture.make_task(
        &run,
        "retry-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Retry task",
        &["src/data.rs"],
        &["src/data.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    retry_task.attempt_count = 1;
    fixture.persist_run_with_tasks(&run, &[retry_task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: retry_task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "scope failed".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "scope failed".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            validation_errors: vec![format!(
                "changed file '{}' is outside declared write scope {:?}",
                "src/data/tests.rs",
                vec!["src/data.rs".to_string()]
            )],
            ..Default::default()
        },
    });

    let claimed = claim_next_runnable_task(&run.run_id, 61, "codex")
        .unwrap()
        .expect("retry task should claim after scope holder finishes");

    assert_eq!(claimed.task_id, retry_task.task_id);
    assert!(claimed
        .write_scope
        .contains(&"src/data/tests.rs".to_string()));
    let persisted = load_task(&run.run_id, &retry_task.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Running);
    assert!(persisted
        .write_scope
        .contains(&"src/data/tests.rs".to_string()));
}

#[test]
fn claim_next_runnable_task_waits_for_active_scope_holder_before_retrying() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut retry_task = fixture.make_task(
        &run,
        "retry-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Retry task",
        &["src/data.rs"],
        &["src/data.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    retry_task.attempt_count = 1;
    let mut owner = fixture.make_task(
        &run,
        "owner",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Owner task",
        &["src/data/tests.rs"],
        &["src/data/tests.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    owner.claimed_by = Some(62);

    fixture.persist_run_with_tasks(&run, &[retry_task.clone(), owner.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: retry_task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "scope failed".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "scope failed".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/data/tests.rs".to_string()],
            validation_errors: vec![format!(
                "changed file '{}' is outside declared write scope {:?}",
                "src/data/tests.rs",
                vec!["src/data.rs".to_string()]
            )],
            ..Default::default()
        },
    });
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: owner.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 62,
        launcher_pid: Some(62),
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

    let claimed = claim_next_runnable_task(&run.run_id, 63, "codex").unwrap();

    assert!(
        claimed.is_none(),
        "retry task should wait for active scope holder"
    );
    let persisted = load_task(&run.run_id, &retry_task.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Queued);
    assert_eq!(persisted.write_scope, vec!["src/data.rs".to_string()]);
}

#[test]
fn release_claim_after_launch_failure_requeues_retryable_task() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "write-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write feature",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(41);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 41,
        launcher_pid: Some(41),
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

    release_claim_after_launch_failure(
        &run.run_id,
        &task.task_id,
        41,
        &anyhow::anyhow!("worker binary missing"),
    )
    .unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert_eq!(reloaded.attempt_count, 1);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn release_claim_after_launch_failure_fails_exhausted_task() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "exhausted-launch",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Exhausted launch",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(51);

    task.attempt_count = 2;
    task.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 51,
        launcher_pid: Some(51),
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

    release_claim_after_launch_failure(
        &run.run_id,
        &task.task_id,
        51,
        &anyhow::anyhow!("launcher failed"),
    )
    .unwrap();

    let failed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(failed.status, CoordinationTaskStatus::Failed);
    assert_eq!(failed.attempt_count, 3);
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn expire_stale_claims_requeues_retryable_task_after_stale_claim() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let stale_since = now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1);
    let mut task = fixture.make_task(
        &run,
        "retryable-expiry",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Retryable expiry",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 1;
    task.max_attempts = 3;
    task.claimed_by = Some(61);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 61,
        launcher_pid: Some(61),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: stale_since,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: stale_since,
    })
    .unwrap());

    expire_stale_claims(&run.run_id).unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn expire_stale_claims_requeues_exhausted_task_after_stale_claim() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let stale_since = now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1);
    let mut task = fixture.make_task(
        &run,
        "exhausted-expiry",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Exhausted expiry",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.attempt_count = 3;
    task.max_attempts = 3;
    task.claimed_by = Some(62);

    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 62,
        launcher_pid: Some(62),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: stale_since,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: stale_since,
    })
    .unwrap());

    expire_stale_claims(&run.run_id).unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert!(reloaded.artifact_summary.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn expire_stale_claims_skips_malformed_files_and_processes_valid_claims() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let stale_since = now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1);
    let mut expired = fixture.make_task(
        &run,
        "expired-claim",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Expired claim",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    expired.claimed_by = Some(71);

    let mut live = fixture.make_task(
        &run,
        "live-claim",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Live claim",
        &["src/main.rs"],
        &["src/main.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    live.claimed_by = Some(72);

    fixture.persist_run_with_tasks(&run, &[expired.clone(), live.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: expired.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 71,
        launcher_pid: Some(71),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: stale_since,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: stale_since,
    })
    .unwrap());
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: live.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 72,
        launcher_pid: Some(72),
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
    fs::write(claims_dir(&run.run_id).join("bad.json"), "{not valid json").unwrap();

    expire_stale_claims(&run.run_id).unwrap();

    let expired_task = load_task(&run.run_id, &expired.task_id).unwrap();
    assert_eq!(expired_task.status, CoordinationTaskStatus::Queued);
    assert!(!claim_path(&run.run_id, &expired.task_id).exists());

    let live_task = load_task(&run.run_id, &live.task_id).unwrap();
    assert_eq!(live_task.status, CoordinationTaskStatus::Running);
    assert!(claim_path(&run.run_id, &live.task_id).exists());
    assert!(claims_dir(&run.run_id).join("bad.json").exists());
}
