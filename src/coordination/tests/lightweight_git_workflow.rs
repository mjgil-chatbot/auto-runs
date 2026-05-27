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

#[test]
fn lightweight_completion_publishes_to_current_auto_runs_branch() {
    let fixture = CoordinationTestFixture::named("lightweight-git-auto-runs-branch");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let remote_main_before = run_git(&remote_path, &["rev-parse", "refs/heads/main"]);

    run_git(
        fixture.workspace.path(),
        &["checkout", "--orphan", "auto-runs-1"],
    );
    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() -> &'static str { \"auto-runs\" }\n",
    )
    .unwrap();

    let task = lightweight_running_write_task(&fixture, &run, "minimax");
    persist_claimed_task(&fixture, &run, &task, "minimax");
    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    assert_eq!(
        run_git(&remote_path, &["show", "refs/heads/auto-runs-1:src/lib.rs"]),
        "pub fn sample() -> &'static str { \"auto-runs\" }"
    );
    assert_eq!(
        run_git(&remote_path, &["rev-parse", "refs/heads/main"]),
        remote_main_before
    );
}

struct SequentialSameFilePublishOutcome {
    remote_log: String,
    remote_commit_message: String,
    remote_final: String,
}

fn run_sequential_same_file_publish_scenario(
    label: &str,
    first_provider: &str,
    first_title: &str,
    first_content: &str,
    second_provider: &str,
    second_title: &str,
    second_content: &str,
) -> SequentialSameFilePublishOutcome {
    let fixture = CoordinationTestFixture::named(label);
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());

    let mut first = lightweight_running_write_task(&fixture, &run, first_provider);
    first.title = first_title.to_string();
    persist_claimed_task(&fixture, &run, &first, first_provider);
    std::fs::write(fixture.workspace.path().join("src/lib.rs"), first_content).unwrap();
    complete_task(
        &run.run_id,
        &first.task_id,
        71,
        lightweight_success_artifact(&run, &first),
    )
    .unwrap();

    let mut second = lightweight_running_write_task(&fixture, &run, second_provider);
    second.task_id = "write-task-2".to_string();
    second.title = second_title.to_string();
    persist_claimed_task(&fixture, &run, &second, second_provider);
    std::fs::write(fixture.workspace.path().join("src/lib.rs"), second_content).unwrap();
    complete_task(
        &run.run_id,
        &second.task_id,
        71,
        lightweight_success_artifact(&run, &second),
    )
    .unwrap();

    SequentialSameFilePublishOutcome {
        remote_log: run_git(&remote_path, &["log", "--pretty=%s", "-2"]),
        remote_commit_message: run_git(&remote_path, &["log", "-1", "--pretty=%B"]),
        remote_final: run_git(&remote_path, &["show", "HEAD:src/lib.rs"]),
    }
}

#[test]
fn lightweight_generated_write_tasks_use_runtime_git_publish() {
    // The task PLANNER (task splitter) is told to set title = exact git commit message.
    // This verifies the planner-side instruction exists so the coordinator can derive
    // the commit message from the task title after the worker completes the slice.
    let fixture = CoordinationTestFixture::named("lightweight-git-prompt");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let seeded = seed_lightweight_implementation_tasks(&run);
    let implementation_plan = seeded
        .iter()
        .find(|task| {
            task.kind == CoordinationTaskKind::PlanFlatTasks
                && task.task_category == Some(TaskCategory::Plan)
        })
        .unwrap();
    // Planner gets the instruction to author the commit message via title
    assert!(implementation_plan
        .prompt
        .contains("exact git commit message"));

    fixture.persist_run_with_tasks(&run, &[implementation_plan.clone()]);
    expand_plan_task(
        &run.run_id,
        implementation_plan,
        r#"[
            {
                "title": "Implement slice",
                "prompt": "Implement the requested slice.",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": ["src"],
                "write_scope": ["src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["slice is implemented", "review can verify the slice without worker task-list edits"]
            }
        ]"#,
    )
    .unwrap();

    let task = load_tasks(&run.run_id)
        .unwrap()
        .into_iter()
        .find(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .unwrap();
    // WORKER: prompt must not reveal git commit message authorship; worker just completes slice
    assert!(!task.prompt.contains("Git workflow:"));
    assert!(!task.prompt.contains("Git commit: <commit-hash>"));
    assert!(!task.prompt.contains("Git push: pushed"));

    // WORKER contract: no git commit/push instructions; coordinator owns commit from title
    let contract = provider_response_contract(&task);
    assert!(!contract.contains("Git commit: <commit-hash>"));
    assert!(!contract.contains("Git push: pushed"));
}

#[test]
fn reconciler_publishes_deferred_lightweight_write_completion() {
    let fixture = CoordinationTestFixture::named("lightweight-git-reconciler-publish");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() -> &'static str { \"reconciled\" }\n",
    )
    .unwrap();

    let mut task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Implement slice",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    task.failure_kind = Some(TaskFailureKind::Publish);
    task.recovery_action = Some(TaskRecoveryAction::RetryPublish);
    task.artifact_summary = Some("implemented slice [completed_with_changes]".to_string());

    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    fixture.write_artifact(&lightweight_success_artifact(&run, &task));
    fixture.write_artifact_materialized_files(
        &run.run_id,
        &task.task_id,
        &[(
            "src/lib.rs",
            "pub fn sample() -> &'static str { \"reconciled\" }\n",
        )],
    );

    reconcile_run_state(&run.run_id).unwrap();

    let task = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(task.recovery_action, None);
    assert_eq!(task.failure_kind, None);
    assert!(task
        .artifact_summary
        .as_deref()
        .unwrap_or_default()
        .contains("Published git commit"));
    assert_eq!(
        run_git(&remote_path, &["show", "HEAD:src/lib.rs"]),
        "pub fn sample() -> &'static str { \"reconciled\" }"
    );
}

// COORDINATOR: patch-level publish applies worker changes to latest branch head,
// commits with task.title as message, and pushes to origin. The fixture workspace
// (worker's sandbox) is not where the commit lands — the remote receives it.
#[test]
fn lightweight_completion_auto_publishes_materialized_changes() {
    let fixture = CoordinationTestFixture::named("lightweight-git-auto-publish");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.title = "Implement slice\nRefresh task list".to_string();
    persist_claimed_task(&fixture, &run, &task, "codex");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"hi\"); }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let persisted = load_task(&run.run_id, &task.task_id).unwrap();
    // Remote (origin) has the new commit from patch-level publish
    let remote_head = run_git(&remote_path, &["rev-parse", "HEAD"]);
    let remote_commit_message = run_git(&remote_path, &["log", "-1", "--pretty=%B"]);

    // Task completed successfully
    assert_eq!(persisted.status, CoordinationTaskStatus::Completed);
    // Remote has the new commit with task.title as message
    assert_eq!(remote_commit_message, "Implement slice\nRefresh task list");
    // The new commit is on origin/main
    assert!(!remote_head.is_empty());
}

// COORDINATOR: bootstrapped workspace still gets commit+push from task title.
#[test]
fn lightweight_completion_auto_publishes_from_bootstrapped_workspace() {
    let fixture = CoordinationTestFixture::named("lightweight-git-bootstrap-publish");
    let (_gh_dir, remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let missing_workspace = fixture.workspace.path().join("fresh-workspace");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    let task = lightweight_running_write_task(&fixture, &run, "codex");
    persist_claimed_task(&fixture, &run, &task, "codex");

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    std::fs::create_dir_all(missing_workspace.join("src")).unwrap();
    std::fs::write(
        missing_workspace.join("src/lib.rs"),
        "pub fn sample() { println!(\"bootstrap\"); }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let persisted = load_task(&run.run_id, &task.task_id).unwrap();
    let local_head = run_git(&missing_workspace, &["rev-parse", "HEAD"]);
    let origin_path = run_git(&missing_workspace, &["remote", "get-url", "origin"]);
    let origin_head = run_git(std::path::Path::new(&origin_path), &["rev-parse", "HEAD"]);
    let remotes = run_git(&missing_workspace, &["remote"]);

    assert_eq!(persisted.status, CoordinationTaskStatus::Completed);
    assert_ne!(local_head, origin_head);
    assert_eq!(remotes, "origin");
    assert!(origin_path.starts_with(remote_root.path().to_str().unwrap()));
}

#[test]
fn lightweight_completion_keeps_remote_publishable_after_sequential_publishes_without_resetting_workspace(
) {
    let fixture = CoordinationTestFixture::named("lightweight-git-sequential-no-reset");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());

    let mut first = lightweight_running_write_task(&fixture, &run, "codex");
    first.title = "First publish".to_string();
    persist_claimed_task(&fixture, &run, &first, "codex");
    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"first\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &first.task_id,
        71,
        lightweight_success_artifact(&run, &first),
    )
    .unwrap();

    let remote_head_after_first = run_git(&remote_path, &["rev-parse", "HEAD"]);
    assert!(!remote_head_after_first.is_empty());

    let mut second = lightweight_running_write_task(&fixture, &run, "codex");
    second.task_id = "write-task-2".to_string();
    second.title = "Second publish".to_string();
    persist_claimed_task(&fixture, &run, &second, "codex");
    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"second\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &second.task_id,
        71,
        lightweight_success_artifact(&run, &second),
    )
    .unwrap();

    let remote_head_after_second = run_git(&remote_path, &["rev-parse", "HEAD"]);
    let remote_commit_message = run_git(&remote_path, &["log", "-1", "--pretty=%B"]);

    assert_ne!(
        remote_head_after_first,
        remote_head_after_second,
        "tasks={:?}; events={:?}",
        load_tasks(&run.run_id).unwrap(),
        load_events(&run.run_id, None, None).unwrap()
    );
    assert_eq!(remote_commit_message, "Second publish");
}

#[test]
fn lightweight_publish_failure_stays_deferred_after_reconcile_attempt() {
    let fixture = CoordinationTestFixture::named("lightweight-git-publish-retry");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let hook_path = remote_path.join("hooks/pre-receive");
    // Test fixtures own their directories even when Git templates are disabled.
    std::fs::create_dir_all(hook_path.parent().unwrap()).unwrap();
    std::fs::write(
        &hook_path,
        "#!/bin/sh\necho 'remote: fatal error in commit_refs' >&2\nexit 1\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&hook_path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&hook_path, permissions).unwrap();
    }

    let mut task = lightweight_running_write_task(&fixture, &run, "minimax");
    task.title = "Repair push blocker".to_string();
    persist_claimed_task(&fixture, &run, &task, "minimax");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"retry me\"); }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    let persisted_run = load_run(&run.run_id).unwrap();

    assert_eq!(reloaded.status, CoordinationTaskStatus::Completed);
    assert_eq!(
        reloaded.recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );
    assert_eq!(reloaded.failure_kind, Some(TaskFailureKind::Publish));
    assert!(reloaded.claimed_by.is_none());
    assert_eq!(persisted_run.status, CoordinationRunStatus::Completed);
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    reconcile_run_state(&run.run_id).unwrap();

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();

    assert_eq!(reloaded.status, CoordinationTaskStatus::Completed);
    assert!(reloaded.claimed_by.is_none());
    assert_eq!(
        reloaded.recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );
    assert!(reloaded.publish_attempt_count >= 1);
    assert!(reloaded
        .artifact_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("Deferred git publish failed")));
    assert!(reloaded
        .artifact_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("remote: fatal error in commit_refs")));
}

/// Regression test: reconcile_deferred_lightweight_publish must not increment
/// publish_attempt_count when it does not actually attempt a publish (e.g., when
/// the reconciler is called multiple times while the run is still active and
/// the task is not the one being reconciled). Multiple reconcile_run_state
/// calls must not exhaust the retry budget prematurely.
#[test]
fn reconcile_deferred_lightweight_publish_does_not_increment_count_on_skipped_calls() {
    let fixture = CoordinationTestFixture::named("lightweight-git-publish-no-premature-increment");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let _remote_path = init_git_repo_with_remote(fixture.workspace.path());

    // Two write tasks: one already completed (RetryPublish), one still running.
    let mut task1 = lightweight_running_write_task(&fixture, &run, "minimax");
    task1.title = "First task".to_string();
    task1.recovery_action = Some(TaskRecoveryAction::RetryPublish);
    task1.publish_attempt_count = 0;
    task1.max_attempts = 3;
    persist_claimed_task(&fixture, &run, &task1, "minimax");

    let mut task2 = lightweight_running_write_task(&fixture, &run, "minimax");
    task2.task_id = "task-two".to_string();
    task2.title = "Second task".to_string();
    persist_claimed_task(&fixture, &run, &task2, "minimax");

    std::fs::write(fixture.workspace.path().join("src/lib.rs"), "fn one() {}\n").unwrap();
    complete_task(
        &run.run_id,
        &task1.task_id,
        71,
        lightweight_success_artifact(&run, &task1),
    )
    .unwrap();

    // Simulate multiple reconcile_run_state calls as would happen from multiple
    // drive_run_for_instance iterations before the task is actually retried.
    reconcile_run_state(&run.run_id).unwrap();
    reconcile_run_state(&run.run_id).unwrap();
    reconcile_run_state(&run.run_id).unwrap();

    let reloaded = load_task(&run.run_id, &task1.task_id).unwrap();

    // publish_attempt_count should be exactly 1 (incremented only after actual
    // publish attempt), not 3+ from the extra reconcile calls.
    assert_eq!(
        reloaded.publish_attempt_count, 1,
        "publish_attempt_count should not be inflated by reconcile calls that skip the task"
    );
    // The initial publish succeeded; later reconciliation has nothing to retry.
    assert_eq!(reloaded.recovery_action, None);
}

#[test]
fn lightweight_completion_merges_same_file_fix_after_broken_publish() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-sequential-fix",
        "codex",
        "Introduce broken sample",
        "pub fn sample() { todo!(\"broken worker output\"); }\n",
        "claude",
        "Fix broken sample",
        "pub fn sample() { println!(\"fixed\"); }\n",
    );

    assert_eq!(
        outcome.remote_log,
        "Fix broken sample\nIntroduce broken sample"
    );
    assert!(outcome.remote_final.contains("println!(\"fixed\")"));
    assert!(!outcome.remote_final.contains("broken worker output"));
}

#[test]
fn lightweight_completion_can_rebreak_same_file_after_a_correct_publish() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-sequential-rebreak",
        "codex",
        "Ship correct sample",
        "pub fn sample() { println!(\"green\"); }\n",
        "minimax",
        "Rebreak sample",
        "pub fn sample() { panic!(\"regression\"); }\n",
    );

    assert_eq!(outcome.remote_log, "Rebreak sample\nShip correct sample");
    assert!(outcome.remote_final.contains("panic!(\"regression\")"));
    assert!(!outcome.remote_final.contains("println!(\"green\")"));
}

#[test]
fn lightweight_completion_preserves_non_overlapping_same_file_changes_across_publishes() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-non-overlap",
        "codex",
        "Add helper function",
        "pub fn sample() { println!(\"first\"); }\n\npub fn helper() { println!(\"helper\"); }\n",
        "claude",
        "Adjust sample output",
        "pub fn sample() { println!(\"second\"); }\n\npub fn helper() { println!(\"helper\"); }\n",
    );

    assert!(outcome.remote_final.contains("println!(\"second\")"));
    assert!(outcome.remote_final.contains("pub fn helper()"));
}

#[test]
fn lightweight_completion_uses_latest_same_line_edit_for_follow_up_publish() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-same-line-overwrite",
        "codex",
        "Write first version",
        "pub fn sample() { println!(\"v1\"); }\n",
        "claude",
        "Write second version",
        "pub fn sample() { println!(\"v2\"); }\n",
    );

    assert!(outcome.remote_final.contains("println!(\"v2\")"));
    assert!(!outcome.remote_final.contains("println!(\"v1\")"));
}

#[test]
fn lightweight_completion_preserves_multiline_commit_message_for_same_file_follow_up() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-multiline-follow-up",
        "codex",
        "Initial sample change",
        "pub fn sample() { println!(\"initial\"); }\n",
        "claude",
        "Fix sample output\nUpdate checklist",
        "pub fn sample() { println!(\"final\"); }\n",
    );

    assert_eq!(
        outcome.remote_commit_message,
        "Fix sample output\nUpdate checklist"
    );
}

#[test]
fn lightweight_completion_keeps_remote_publish_order_stable_after_mixed_provider_sequence() {
    let outcome = run_sequential_same_file_publish_scenario(
        "lightweight-git-provider-mix",
        "gemini",
        "First provider change",
        "pub fn sample() { println!(\"one\"); }\n",
        "minimax",
        "Second provider change",
        "pub fn sample() { println!(\"two\"); }\n",
    );

    assert_eq!(
        outcome.remote_log,
        "Second provider change\nFirst provider change"
    );
}

#[test]
fn lightweight_completion_limits_publish_to_materialized_files() {
    let fixture = CoordinationTestFixture::named("lightweight-git-materialized-only");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.title = "Publish materialized files only".to_string();
    task.write_scope = vec!["src".to_string()];
    persist_claimed_task(&fixture, &run, &task, "codex");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"updated\"); }\n",
    )
    .unwrap();
    std::fs::write(
        fixture.workspace.path().join("src/helper.rs"),
        "pub fn helper() -> &'static str { \"leftover\" }\n",
    )
    .unwrap();

    let mut artifact = lightweight_success_artifact(&run, &task);
    artifact.report.changed_files = vec!["src/lib.rs".to_string()];
    complete_task(&run.run_id, &task.task_id, 71, artifact).unwrap();

    assert_eq!(
        run_git(&remote_path, &["show", "HEAD:src/lib.rs"]),
        "pub fn sample() { println!(\"updated\"); }"
    );
    let remote_tree = run_git(&remote_path, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(!remote_tree.lines().any(|line| line == "src/helper.rs"));
}

#[test]
fn lightweight_completion_excludes_build_outputs_and_large_untracked_files_from_publish() {
    let fixture = CoordinationTestFixture::named("lightweight-git-publish-filter");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.title = "Publish source only".to_string();
    task.write_scope = vec!["**".to_string()];
    persist_claimed_task(&fixture, &run, &task, "codex");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"updated\"); }\n",
    )
    .unwrap();
    std::fs::create_dir_all(fixture.workspace.path().join("target/release")).unwrap();
    std::fs::write(
        fixture.workspace.path().join("target/release/build.log"),
        "do not publish build outputs\n",
    )
    .unwrap();
    std::fs::write(
        fixture.workspace.path().join("tmp-dump.log"),
        "x".repeat((2 * 1024 * 1024) + 128),
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let remote_tree = run_git(&remote_path, &["ls-tree", "-r", "--name-only", "HEAD"]);
    assert!(remote_tree.lines().any(|line| line == "src/lib.rs"));
    assert!(!remote_tree.lines().any(|line| line.starts_with("target/")));
    assert!(!remote_tree.lines().any(|line| line == "tmp-dump.log"));
}

// COORDINATOR: lightweight write completion is recorded first; failed publish remains deferred until repaired or retried.
#[test]
fn lightweight_completion_keeps_retry_publish_state_when_reconciler_cannot_push() {
    let fixture = CoordinationTestFixture::named("lightweight-git-auto-publish-fail");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    init_git_repo_with_remote(fixture.workspace.path());
    run_git(
        fixture.workspace.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "/definitely/missing/auto-runs-remote.git",
        ],
    );
    let task = lightweight_running_write_task(&fixture, &run, "codex");
    persist_claimed_task(&fixture, &run, &task, "codex");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"hi\"); }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let completed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);
    assert_eq!(
        completed.recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );
    assert_eq!(completed.failure_kind, Some(TaskFailureKind::Publish));

    reconcile_run_state(&run.run_id).unwrap();

    let recovered = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(recovered.status, CoordinationTaskStatus::Completed);
    assert_eq!(
        recovered.recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );
    assert_eq!(recovered.failure_kind, Some(TaskFailureKind::Publish));
    assert!(recovered.publish_attempt_count >= 1);
    assert!(recovered
        .artifact_summary
        .as_deref()
        .is_some_and(|summary| summary.contains("Deferred git publish failed")));
}

#[test]
fn lightweight_publish_repair_task_does_not_spawn_nested_publish_repair() {
    let fixture = CoordinationTestFixture::named("lightweight-git-no-nested-publish-repair");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    init_git_repo_with_remote(fixture.workspace.path());
    run_git(
        fixture.workspace.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "/definitely/missing/auto-runs-remote.git",
        ],
    );

    let mut task = fixture.make_task_with_options(
        &run,
        "repair-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Repair git publish for Implement slice",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    task.task_category = Some(TaskCategory::DeepWrite);
    task.failure_kind = Some(TaskFailureKind::Publish);
    task.recovery_action = Some(TaskRecoveryAction::RetryPublish);
    task.max_attempts = 1;
    task.artifact_summary = Some("repair publish deferred".to_string());

    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    fixture.write_artifact(&lightweight_success_artifact(&run, &task));
    fixture.write_artifact_materialized_files(
        &run.run_id,
        &task.task_id,
        &[("src/lib.rs", "pub fn sample() { println!(\"repair\"); }\n")],
    );

    reconcile_run_state(&run.run_id).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    assert_eq!(
        tasks
            .iter()
            .filter(|candidate| {
                candidate
                    .title
                    .starts_with("Repair git publish for Repair git publish for ")
            })
            .count(),
        0
    );
}

#[test]
fn publish_tracks_its_base_preserving_head_index_and_workspace() {
    let fixture = CoordinationTestFixture::named("publish-preserves-index-and-workspace");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote = init_git_repo_with_remote(fixture.workspace.path());
    fs::write(
        fixture.workspace.path().join("staged-note.txt"),
        "staged content\n",
    )
    .unwrap();
    run_git(fixture.workspace.path(), &["add", "staged-note.txt"]);
    let index_before = run_git(fixture.workspace.path(), &["write-tree"]);
    let head_before = run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    fs::write(
        fixture.workspace.path().join("untracked-note.txt"),
        "pending content\n",
    )
    .unwrap();
    let task = lightweight_running_write_task(&fixture, &run, "codex");
    persist_claimed_task(&fixture, &run, &task, "codex");
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"published\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();
    let mut follow_up = lightweight_running_write_task(&fixture, &run, "codex");
    follow_up.task_id = "second-publish".to_string();
    persist_claimed_task(&fixture, &run, &follow_up, "codex");
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"published-again\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &follow_up.task_id,
        71,
        lightweight_success_artifact(&run, &follow_up),
    )
    .unwrap();
    assert!(run_git(&remote, &["show", "HEAD:src/lib.rs"]).contains("published-again"));
    assert_eq!(
        run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]),
        head_before
    );
    assert_eq!(
        run_git(fixture.workspace.path(), &["write-tree"]),
        index_before
    );
    assert_eq!(
        fs::read_to_string(fixture.workspace.path().join("untracked-note.txt")).unwrap(),
        "pending content\n"
    );
    assert!(
        fs::read_to_string(fixture.workspace.path().join("src/lib.rs"))
            .unwrap()
            .contains("published")
    );
    assert!(!run_git(&remote, &["ls-tree", "--name-only", "HEAD"]).contains("staged-note.txt"));
}

#[test]
fn publish_preserves_a_divergent_local_branch_head() {
    let fixture = CoordinationTestFixture::named("publish-preserves-local-commits");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote = init_git_repo_with_remote(fixture.workspace.path());
    fs::write(
        fixture.workspace.path().join("local-note.txt"),
        "local-only commit\n",
    )
    .unwrap();
    run_git(fixture.workspace.path(), &["add", "local-note.txt"]);
    run_git(
        fixture.workspace.path(),
        &["commit", "-m", "local-only commit"],
    );
    let local_head = run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    let task = lightweight_running_write_task(&fixture, &run, "codex");
    persist_claimed_task(&fixture, &run, &task, "codex");
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"published\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();
    assert_eq!(
        run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]),
        local_head
    );
    assert!(run_git(&remote, &["show", "HEAD:src/lib.rs"]).contains("published"));
}

#[test]
fn publish_rejects_external_conflict_after_unrelated_local_publish() {
    let fixture = CoordinationTestFixture::named("publish-external-conflict-boundary");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let remote = init_git_repo_with_remote(fixture.workspace.path());
    let external = tempfile::tempdir().unwrap();
    run_git(
        fixture.workspace.path(),
        &[
            "clone",
            remote.to_str().unwrap(),
            external.path().to_str().unwrap(),
        ],
    );
    run_git(external.path(), &["config", "user.name", "External Writer"]);
    run_git(
        external.path(),
        &["config", "user.email", "external@example.com"],
    );
    fs::write(
        external.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"external\"); }\n",
    )
    .unwrap();
    run_git(external.path(), &["add", "src/lib.rs"]);
    run_git(external.path(), &["commit", "-m", "external change"]);
    run_git(external.path(), &["push", "origin", "main"]);
    let mut note = lightweight_running_write_task(&fixture, &run, "codex");
    note.task_id = "unrelated-note".to_string();
    note.write_scope = vec!["note.txt".to_string()];
    persist_claimed_task(&fixture, &run, &note, "codex");
    fs::write(fixture.workspace.path().join("note.txt"), "local note\n").unwrap();
    let mut artifact = lightweight_success_artifact(&run, &note);
    artifact.report.changed_files = vec!["note.txt".to_string()];
    complete_task(&run.run_id, &note.task_id, 71, artifact).unwrap();
    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.task_id = "conflicting-source".to_string();
    persist_claimed_task(&fixture, &run, &task, "codex");
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"local conflicting change\"); }\n",
    )
    .unwrap();
    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();
    assert!(run_git(&remote, &["show", "HEAD:src/lib.rs"]).contains("external"));
    assert_eq!(
        load_task(&run.run_id, &task.task_id)
            .unwrap()
            .recovery_action,
        Some(TaskRecoveryAction::RetryPublish)
    );
}
