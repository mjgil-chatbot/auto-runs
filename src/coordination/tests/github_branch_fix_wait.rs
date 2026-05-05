fn run_git_for_branch_fix_test(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn init_git_repo_for_branch_fix_test(workspace: &std::path::Path) {
    run_git_for_branch_fix_test(workspace, &["init", "-b", "main"]);
    run_git_for_branch_fix_test(workspace, &["config", "user.name", "Test User"]);
    run_git_for_branch_fix_test(workspace, &["config", "user.email", "test@example.com"]);
    run_git_for_branch_fix_test(workspace, &["add", "."]);
    run_git_for_branch_fix_test(workspace, &["commit", "-m", "Initial commit"]);
}

#[test]
fn github_branch_fix_wait_task_requeues_while_success_is_still_pending() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-wait-requeue");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "build", 5);
    let wait_spec = github_branch_fix_wait_for_success_spec(&run);
    let mut wait_task = fixture.make_task_with_options(
        &run,
        "wait-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("minimax"),
        &wait_spec.title,
        wait_spec.allowed_tools.clone(),
        wait_spec.read_scope.clone(),
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    wait_task.prompt = wait_spec.prompt.clone();
    wait_task.acceptance_checks = wait_spec.acceptance_checks.clone();
    wait_task.task_category = Some(TaskCategory::Research);
    wait_task.attempt_count = 1;
    wait_task.claimed_by = Some(88);
    fixture.persist_run_with_tasks(&run, &[wait_task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: wait_task.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 88,
        launcher_pid: Some(88),
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
        task_id: wait_task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::Research),
        fallback_from: None,
        summary: "WAITING_FOR_SUCCESS: latest build run 25252864863 is still in_progress for headSha 6da2a7b".to_string(),
        body: "NO_CHANGE: WAITING_FOR_SUCCESS: latest build run 25252864863 is still in_progress for headSha 6da2a7b".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "WAITING_FOR_SUCCESS: latest build run 25252864863 is still in_progress for headSha 6da2a7b".to_string(),
            details: "WAITING_FOR_SUCCESS: no successful completed run exists yet for headSha 6da2a7b".to_string(),
            ..Default::default()
        },
    };

    complete_task_with_materialized_files(
        &run.run_id,
        &wait_task.task_id,
        88,
        artifact,
        Vec::new(),
    )
    .unwrap();

    let queued = load_task(&run.run_id, &wait_task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.attempt_count, 0);
    assert!(!claim_path(&run.run_id, &wait_task.task_id).exists());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event
                .message
                .contains("wait task returned to queue without consuming attempt budget")
    }));
}

#[test]
fn github_branch_fix_validation_read_requeues_while_success_is_still_pending() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-validation-requeue");
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    init_git_repo_for_branch_fix_test(fixture.workspace.path());
    let head = run_git_for_branch_fix_test(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    let _run_list_guard = EnvVarGuard::set(
        "AUTO_RUNS_TEST_GH_RUN_LIST_STDOUT",
        format!(r#"[{{"status":"in_progress","conclusion":null,"headSha":"{head}"}}]"#),
    );

    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "build", 5);
    let mut task = fixture.make_task_with_options(
        &run,
        "validate-success-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("minimax"),
        "Validate GitHub Actions run completed successfully after fix push",
        vec!["filesystem".to_string(), "search".to_string()],
        vec![".github/workflows".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.prompt = "Run `gh run list --branch build --limit 5` and confirm whether any run completed successfully after the most recent fix push.".to_string();
    task.acceptance_checks = vec![
        "gh run list returns successfully".to_string(),
        "successful run identified or confirmed none exists post-fix".to_string(),
    ];
    task.task_category = Some(TaskCategory::Research);
    task.attempt_count = 1;
    task.claimed_by = Some(88);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 88,
        launcher_pid: Some(88),
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
        task_id: task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::Research),
        fallback_from: None,
        summary: "COMPLETED: no successful run exists yet".to_string(),
        body: "COMPLETED: latest build run is still in_progress.".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "COMPLETED: no successful run exists yet".to_string(),
            details: "latest build run is still in_progress".to_string(),
            ..Default::default()
        },
    };

    complete_task_with_materialized_files(&run.run_id, &task.task_id, 88, artifact, Vec::new())
        .unwrap();

    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.attempt_count, 0);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event.message.contains(
                "Validate GitHub Actions run completed successfully after fix push",
            )
    }));
}
