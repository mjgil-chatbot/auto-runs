fn run_git(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} should succeed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
fn init_git_repo(workspace: &std::path::Path) {
    run_git(workspace, &["init", "-b", "main"]);
    run_git(workspace, &["config", "user.name", "Test User"]);
    run_git(workspace, &["config", "user.email", "test@example.com"]);
    run_git(workspace, &["add", "src/lib.rs"]);
    run_git(workspace, &["commit", "-m", "initial"]);
}
fn init_git_repo_with_remote(workspace: &std::path::Path) -> std::path::PathBuf {
    let remote_path = workspace.join(".test-remote.git");
    std::fs::create_dir_all(&remote_path).unwrap();
    run_git(&remote_path, &["init", "--bare"]);
    init_git_repo(workspace);
    run_git(
        workspace,
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    );
    run_git(workspace, &["push", "-u", "origin", "main"]);
    // Ensure the bare repo's HEAD points to main
    run_git(&remote_path, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    remote_path
}
fn lightweight_running_write_task(
    fixture: &CoordinationTestFixture,
    run: &CoordinationRun,
    provider: &str,
) -> CoordinationTask {
    let mut task = fixture.make_task_with_options(
        run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some(provider),
        "Implement slice",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    task.claimed_by = Some(71);

    task
}
fn persist_claimed_task(
    fixture: &CoordinationTestFixture,
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: &str,
) {
    fixture.persist_run_with_tasks(run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: provider.to_string(),
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
}
fn lightweight_success_artifact(run: &CoordinationRun, task: &CoordinationTask) -> TaskArtifact {
    TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: task.assigned_provider.clone(),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "implemented slice".to_string(),
        body: "COMPLETED: implemented slice".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "implemented slice".to_string(),
            details: "COMPLETED: implemented slice".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            ..Default::default()
        },
    }
}
