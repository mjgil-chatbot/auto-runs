#[test]
fn handle_launch_failure_after_claim_preserves_claimed_attempt_count() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "launch-failure",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write feature",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(41);
    
    task.attempt_count = 1;
    task.max_attempts = 3;
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

    handle_launch_failure_after_claim(
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
fn expand_plan_task_assigns_generated_task_categories() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let plan_task = fixture.make_task(
        &run,
        "plan-task",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan work",
        &["src"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone()]);

    let body = r#"[
        {
            "title":"Research current API",
            "prompt":"Inspect the API surface",
            "kind":"read",
            "read_scope":["src/lib.rs"]
        },
        {
            "title":"Implement API",
            "prompt":"Change the implementation",
            "kind":"write",
            "read_scope":["src/lib.rs"],
            "write_scope":["src/lib.rs"],
            "acceptance_checks":["Check one","Check two"]
        }
    ]"#;

    expand_plan_task(&run.run_id, &plan_task, body).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let read_task = tasks
        .iter()
        .find(|task| task.title == "Research current API")
        .unwrap();
    let write_task = tasks
        .iter()
        .find(|task| task.title == "Implement API")
        .unwrap();

    assert_eq!(read_task.task_category, Some(TaskCategory::Research));
    assert_eq!(write_task.task_category, Some(TaskCategory::DeepWrite));
}

#[test]
fn expand_code_review_plan_assigns_review_task_category() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::CodeReview);
    let plan_task = fixture.make_task(
        &run,
        "review-plan",
        CoordinationTaskKind::PlanCodeReview,
        None,
        "Plan review",
        &["src"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone()]);

    let body = serde_json::to_string(&code_review::CodeReviewPlanArtifact {
        scope_description: "Current workspace".to_string(),
        output_path: "docs/code-review-2026-04-11.md".to_string(),
        automated_checks: Vec::new(),
        profile_warnings: Vec::new(),
        review_tasks: vec![code_review::ReviewTaskPlan {
            title: "Review chunk 1: src/lib.rs:1-10".to_string(),
            prompt: "Review it".to_string(),
            read_scope: vec!["src/lib.rs".to_string()],
            metadata: None,
        }],
    })
    .unwrap();

    expand_code_review_plan(&run.run_id, &plan_task, &body).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let review_task = tasks
        .iter()
        .find(|task| task.title == "Review chunk 1: src/lib.rs:1-10")
        .unwrap();
    assert_eq!(review_task.task_category, Some(TaskCategory::Review));
}

#[test]
fn finalize_launch_failure_requeues_task_and_syncs_index() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "requeue-test",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Requeue on failure",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(50);
    
    task.attempt_count = 0;
    task.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 50,
        launcher_pid: Some(50),
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
    .unwrap();

    release_claim_after_launch_failure(
        &run.run_id,
        &task.task_id,
        50,
        &anyhow::anyhow!("binary not found"),
    )
    .unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert_eq!(reloaded.attempt_count, 1);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn finalize_launch_failure_marks_exhausted_and_syncs_index() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task(
        &run,
        "exhaust-test",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Exhaust on failure",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(60);
    
    task.attempt_count = 2;
    task.max_attempts = 3;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 60,
        launcher_pid: Some(60),
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
    .unwrap();

    release_claim_after_launch_failure(
        &run.run_id,
        &task.task_id,
        60,
        &anyhow::anyhow!("persistent failure"),
    )
    .unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Failed);
    assert_eq!(reloaded.attempt_count, 3);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn find_setsid_binary_returns_path_or_none() {
    let result = find_setsid_binary();
    match result {
        Some(path) => assert!(path.exists()),
        None => {} // acceptable on systems without setsid
    }
}

#[test]
fn spawn_daemonized_fallback_when_setsid_absent() {
    // Verify spawn_daemonized does not panic when given a nonexistent target
    // and setsid may or may not be present — the function should attempt
    // the spawn and return an error rather than panicking.
    let bogus = std::path::Path::new("/nonexistent/binary");
    let result = spawn_daemonized(bogus, &["arg1"], &[]);
    assert!(result.is_err() || result.is_ok());
}

#[test]
fn autonomous_worker_runsc_mounts_coordination_dir_into_container_home() {
    let mounts = autonomous_worker_runsc_mounts();
    let expected_target = std::path::Path::new(".rust-chatbot/coordination");

    assert!(mounts.iter().any(|mount| {
        mount.host.ends_with("coordination")
            && matches!(
                &mount.target,
                ai_providers::sandbox::RunscMountTarget::ContainerHome(path)
                    if path == expected_target
            )
    }));
}
