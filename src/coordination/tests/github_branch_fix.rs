#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[test]
fn github_branch_fix_notes_require_branch() {
    let notes = github_branch_fix_notes_with_settings("", "", 5);
    let error =
        validate_github_branch_fix_notes(&notes).expect_err("branchless config should fail");
    assert!(error.to_string().contains("github_branch_fix_branch"));
}
#[test]
fn github_branch_fix_seeds_branch_specific_single_worker_prompts() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-seeding");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.provider = "gemini".to_string();
    run.notes = github_branch_fix_notes_with_settings("", "feature/ci-fix", 7);

    let seeded = seed_initial_tasks(&run);
    let implementation_plan = seeded
        .iter()
        .find(|task| task.title == "Split GitHub branch fix tasks")
        .expect("implementation plan should exist");
    let validation_plan = seeded
        .iter()
        .find(|task| task.title == "Split GitHub branch fix validation tasks")
        .expect("validation plan should exist");
    let review = seeded
        .iter()
        .find(|task| task.title == "Validate GitHub branch fix pass")
        .expect("review task should exist");

    assert_eq!(
        implementation_plan.assigned_provider.as_deref(),
        Some("gemini")
    );
    assert_eq!(validation_plan.assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(review.assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(implementation_plan.estimated_minutes, Some(60));
    assert_eq!(validation_plan.estimated_minutes, Some(60));
    assert!(implementation_plan
        .prompt
        .contains("Target branch: `feature/ci-fix`"));
    assert!(implementation_plan.prompt.contains("every 7 minute(s)"));
    assert!(implementation_plan.prompt.contains(".github/workflows/"));
    assert!(implementation_plan.prompt.contains("pre-commit hook"));
    assert!(implementation_plan
        .prompt
        .contains("gh auth status"));
    assert!(implementation_plan.prompt.contains("GH_TOKEN"));
    assert!(implementation_plan
        .prompt
        .contains("newer GitHub Actions run for `feature/ci-fix` completed successfully"));
    assert!(!implementation_plan
        .prompt
        .contains("Current task list path"));
    assert!(!implementation_plan.prompt.contains("task list document"));
    assert!(validation_plan
        .prompt
        .contains("latest fix work included relevant test coverage"));
    assert!(review
        .prompt
        .contains("mark the work as complete only after explicitly citing"));
    assert!(review.prompt.contains("gh auth status"));
    assert!(review.prompt.contains("feature/ci-fix"));
    assert!(!review.prompt.contains("task list"));
}
#[test]
fn github_branch_fix_generated_tasks_ignore_provider_overrides() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-provider");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.provider = "claude".to_string();
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    fixture.persist_run_with_tasks(&run, &[]);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("claude"),
        "Split GitHub branch fix tasks",
        vec![
            "filesystem".to_string(),
            "search".to_string(),
            "edit".to_string(),
        ],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&plan_task));

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "Repair CI workflow",
                "provider": "codex",
                "prompt": "Fix the failing workflow.",
                "kind": "write",
                "estimated_minutes": 20,
                "depends_on_titles": [],
                "read_scope": ["src"],
                "write_scope": [".github/workflows/ci.yml"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["workflow covers the target branch", "a newer run completes successfully after push"]
            }
        ]"#,
    )
    .unwrap();

    let generated = load_tasks(&run.run_id)
        .unwrap()
        .into_iter()
        .find(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .expect("generated task should exist");
    assert_eq!(generated.assigned_provider.as_deref(), Some("claude"));
    assert!(generated.prompt.contains("gh auth status"));
    assert!(generated.prompt.contains("gh api rate_limit"));
}
#[test]
fn github_branch_fix_generated_write_contract_uses_lightweight_plaintext() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-contract");
    let run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "github-branch-fix-write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Repair CI workflow",
        &[".github/workflows", "src"],
        &[".github/workflows/ci.yml", "src/lib.rs"],
        &["workflow updated", "tests added"],
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("Return concise plain text only"));
    assert!(contract.contains("COMPLETED:"));
    assert!(!contract.contains("validate_autonomous_task_report.py"));
}
#[test]
fn github_branch_fix_plan_contract_uses_plain_task_array() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-plan-contract");
    let run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "github-branch-fix-plan",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split GitHub branch fix tasks",
        &["**", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("top-level response must be a JSON array of task objects"));
    assert!(contract.contains("first non-whitespace response character must be `[`"));
    assert!(contract.contains("Each write task must include at least 2 concrete `acceptance_checks` strings"));
    assert!(contract.contains("Set `read_scope` to `[\"**\"]` for every generated task"));
    assert!(contract.contains("Set `write_scope` to `[\"**\"]` for every write task"));
    assert!(contract.contains("Patch-level git publish is the consolidation boundary"));
    assert!(!contract.contains("typed task artifact envelope"));
    assert!(!contract.contains("payload_kind"));
    assert!(!contract.contains("validate_autonomous_task_report.py"));
}
#[test]
fn github_branch_fix_plan_tasks_skip_typed_contract_lookup() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-plan-contract-lookup");
    let run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    fixture.persist_run_with_tasks(&run, &[]);
    let plan_task = fixture.make_task(
        &run,
        "github-branch-fix-plan-lookup",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split GitHub branch fix tasks",
        &["**", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    let review_task = fixture.make_task(
        &run,
        "github-branch-fix-review-lookup",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate GitHub branch fix pass",
        &["**", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    assert!(lookup_contract_for_task(&plan_task).is_none());
    assert!(lookup_contract_for_task(&review_task).is_some());
}
#[test]
fn github_branch_fix_review_contract_drops_task_list_requirements() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-review-contract");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "github-branch-fix-review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate GitHub branch fix pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[],
        &["validation-plan"],
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("next branch-fix tasks"));
    assert!(contract.contains("Do not include a `changed_files` field"));
    assert!(!contract.contains("task-list document"));
    assert!(!contract.contains("next task-list"));
}
#[test]
fn github_branch_fix_partial_review_with_failed_ci_synthesizes_wait_loop() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-partial-review-loop");
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    init_git_repo_for_branch_fix_test(fixture.workspace.path());
    let head = run_git_for_branch_fix_test(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    // CI is Failed (completed with failure) — the wait task should be synthesized
    let _run_list_guard = EnvVarGuard::set(
        "AUTO_RUNS_TEST_GH_RUN_LIST_STDOUT",
        format!(r#"[{{"status":"completed","conclusion":"failure","headSha":"{head}"}}]"#),
    );
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "build", 5);
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("minimax"),
        "Follow-up review after Validate GitHub branch fix pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[],
        &["wait-task", "write-task"],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&review_task));

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "CI failed".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "CI failed".to_string(),
            details: "GitHub Actions run completed with failure".to_string(),
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let wait_task = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::GeneratedReadTask)
        .expect("synthesized wait task should exist when CI failed");
    let follow_up_review = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .expect("next follow-up review should exist");

    assert_eq!(
        wait_task.title,
        "Wait for GitHub Actions build run to complete successfully"
    );
    assert!(wait_task.prompt.contains("gh run list --branch build"));
    assert_eq!(wait_task.status, CoordinationTaskStatus::Blocked);
    assert_eq!(follow_up_review.dependencies, vec![wait_task.task_id.clone()]);
    assert_eq!(follow_up_review.status, CoordinationTaskStatus::Blocked);
}
fn github_branch_fix_workspace_bootstrap_recreates_missing_worktree() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-workspace");
    init_git_repo(fixture.workspace.path());
    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "repair branch failures".to_string(),
        workflow_mode: AutonomousWorkflowMode::GithubBranchFix,
        design_doc_path: "docs/github-branch-fix.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: github_branch_fix_notes_with_settings("", "feature/ci-fix", 5),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("branch fix run should create");
    let worktree = std::path::PathBuf::from(snapshot.run.workspace_path.clone());
    let task = snapshot
        .tasks
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::SurveyWorkspace)
        .cloned()
        .expect("survey task should exist");

    std::fs::remove_dir_all(&worktree).unwrap();

    assert!(ensure_task_workspace_exists(&snapshot.run, &task, Some("codex")).unwrap());
    assert!(worktree.is_dir());
    assert_eq!(
        run_git_for_test(&worktree, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feature/ci-fix"
    );
}
#[test]
fn github_branch_fix_upsert_accepts_existing_linked_worktree_as_is() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-existing-worktree");
    init_git_repo(fixture.workspace.path());
    let worktree = fixture
        .workspace
        .path()
        .parent()
        .unwrap()
        .join(format!("github-branch-fix-linked-{}", Uuid::new_v4()));
    run_git(
        fixture.workspace.path(),
        &[
            "worktree",
            "add",
            "--force",
            "-b",
            "feature/ci-fix",
            worktree.to_str().unwrap(),
            "HEAD",
        ],
    );

    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: worktree.display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "repair branch failures".to_string(),
        workflow_mode: AutonomousWorkflowMode::GithubBranchFix,
        design_doc_path: "docs/github-branch-fix.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: github_branch_fix_notes_with_settings("", "feature/ci-fix", 5),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("branch fix run should accept an existing linked worktree");

    assert_eq!(snapshot.run.workspace_path, worktree.display().to_string());
    assert_eq!(
        github_branch_fix_source_workspace(&snapshot.run).as_deref(),
        Some(fixture.workspace.path().to_str().unwrap())
    );
    assert_eq!(
        github_branch_fix_worktree_path(&snapshot.run).as_deref(),
        Some(worktree.to_str().unwrap())
    );
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.cwd == snapshot.run.workspace_path));
}
#[test]
fn github_branch_fix_upsert_accepts_note_only_config_and_forces_one_agent() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-upsert");
    init_git_repo(fixture.workspace.path());
    let snapshot = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "gemini".to_string(),
        goal: "repair branch failures".to_string(),
        workflow_mode: AutonomousWorkflowMode::GithubBranchFix,
        design_doc_path: "docs/github-branch-fix.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 7,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: github_branch_fix_notes_with_settings("", "release/candidate", 9),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("branch fix run should create");

    assert_eq!(snapshot.run.provider, "gemini");
    assert_eq!(snapshot.run.max_agents, 1);
    assert_eq!(
        github_branch_fix_source_workspace(&snapshot.run).as_deref(),
        Some(fixture.workspace.path().to_str().unwrap())
    );
    assert!(snapshot.run.workspace_path.starts_with(&format!(
        "{}-release-candidate-",
        fixture.workspace.path().display()
    )));
    assert!(std::path::Path::new(&snapshot.run.workspace_path).is_dir());
    assert_eq!(
        run_git_for_test(
            std::path::Path::new(&snapshot.run.workspace_path),
            &["rev-parse", "--abbrev-ref", "HEAD"]
        ),
        "release/candidate"
    );
    assert!(snapshot
        .tasks
        .iter()
        .all(|task| task.cwd == snapshot.run.workspace_path));
}
#[test]
fn github_branch_fix_runs_on_same_branch_get_distinct_worktrees() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-run-isolation");
    init_git_repo(fixture.workspace.path());

    let first = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat-1".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "repair branch failures".to_string(),
        workflow_mode: AutonomousWorkflowMode::GithubBranchFix,
        design_doc_path: "docs/github-branch-fix.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: github_branch_fix_notes_with_settings("", "main", 5),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("first branch fix run should create");

    let second = upsert_run(UpsertRunInput {
        existing_run_id: None,
        root_chat_id: "chat-2".to_string(),
        workspace_path: fixture.workspace.path().display().to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "repair branch failures again".to_string(),
        workflow_mode: AutonomousWorkflowMode::GithubBranchFix,
        design_doc_path: "docs/github-branch-fix.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 2,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: github_branch_fix_notes_with_settings("", "main", 5),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
    })
    .expect("second branch fix run should create");

    assert_ne!(first.run.workspace_path, second.run.workspace_path);
    assert!(first.run.workspace_path.starts_with(&format!(
        "{}-main-",
        fixture.workspace.path().display()
    )));
    assert!(second.run.workspace_path.starts_with(&format!(
        "{}-main-",
        fixture.workspace.path().display()
    )));
    assert!(std::path::Path::new(&first.run.workspace_path).is_dir());
    assert!(std::path::Path::new(&second.run.workspace_path).is_dir());
}
#[test]
fn github_branch_fix_poll_timeout_requeues_without_consuming_attempt_budget() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-timeout-retry");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.provider = "minimax".to_string();
    run.auto_retry_failed_tasks = false;
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    let mut task = fixture.make_task(
        &run,
        "github-branch-fix-poll",
        CoordinationTaskKind::GeneratedReadTask,
        Some("minimax"),
        "Poll GitHub Actions for main branch failures",
        &[".github/workflows", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    let now = Utc::now();
    task.attempt_count = 2;
    task.max_attempts = 3;
    task.claimed_by = Some(71);
    
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 71,
        launcher_pid: Some(71),
        lifecycle_phase: ClaimLifecyclePhase::WorkerAdopted,
        worker_pid: Some(71),
        claimed_at: now,
        worker_spawned_at: Some(now),
        worker_adopted_at: Some(now),
        execution_started_at: Some(now),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    })
    .unwrap());

    assert!(requeue_task_for_provider_timeout(
        &run.run_id,
        &task.task_id,
        71,
        "minimax",
        &provider_timeout_failure("minimax", std::time::Duration::from_secs(3 * 60)),
    )
    .unwrap());

    let reloaded = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(reloaded.status, CoordinationTaskStatus::Queued);
    assert_eq!(reloaded.attempt_count, 1);
    assert!(reloaded.claimed_by.is_none());
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}
#[test]
fn github_branch_fix_input_rail_rejects_unauthenticated_gh_cli() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-gh-auth");
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let _auth_guard = EnvVarGuard::set("AUTO_RUNS_TEST_GH_AUTH", "fail");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    let task = fixture.make_task(
        &run,
        "github-branch-fix-poll",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Poll GitHub Actions for main branch failures",
        &[".github/workflows", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let decision = check_input_rail(&run, &task);
    let RailDecision::Reject { reason } = decision else {
        panic!("expected input rail to reject unauthenticated gh cli");
    };
    assert!(reason.contains("authenticated gh CLI access"));
    assert!(reason.contains("gh auth status"));
}
#[test]
fn github_branch_fix_input_rail_falls_back_when_active_flag_is_unsupported() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-gh-auth-fallback");
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let _unsupported_guard = EnvVarGuard::set("AUTO_RUNS_TEST_GH_ACTIVE_UNSUPPORTED", "fail");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    let task = fixture.make_task(
        &run,
        "github-branch-fix-poll",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Poll GitHub Actions for main branch failures",
        &[".github/workflows", "artifacts/**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    assert!(matches!(check_input_rail(&run, &task), RailDecision::Allow));
}
#[test]
fn github_branch_fix_completion_auto_publishes_materialized_changes() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-publish");
    let run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.title = "Repair CI workflow".to_string();
    persist_claimed_task(&fixture, &run, &task, "codex");

    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"github branch fix\"); }\n",
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
    let local_head = run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    let remote_head = run_git(&remote_path, &["rev-parse", "refs/heads/main"]);

    assert_eq!(persisted.status, CoordinationTaskStatus::Completed);
    assert_eq!(local_head, remote_head);
}
#[test]
fn github_branch_fix_publish_recovery_can_push_existing_local_commit() {
    let fixture = CoordinationTestFixture::named("github-branch-fix-publish-resume");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    let remote_path = init_git_repo_with_remote(fixture.workspace.path());
    std::fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"already committed locally\"); }\n",
    )
    .unwrap();
    run_git(fixture.workspace.path(), &["add", "src/lib.rs"]);
    run_git(
        fixture.workspace.path(),
        &["commit", "-m", "Repair CI workflow"],
    );

    let mut task = lightweight_running_write_task(&fixture, &run, "codex");
    task.title = "Repair CI workflow".to_string();
    task.prompt = github_branch_fix_publish_failure_prompt(
        &task.prompt,
        "implemented slice [git_publish_failed]. Automatic git publish failed: git push -u origin HEAD failed",
    );
    persist_claimed_task(&fixture, &run, &task, "codex");

    complete_task(
        &run.run_id,
        &task.task_id,
        71,
        lightweight_success_artifact(&run, &task),
    )
    .unwrap();

    let persisted = load_task(&run.run_id, &task.task_id).unwrap();
    let local_head = run_git(fixture.workspace.path(), &["rev-parse", "HEAD"]);
    let remote_head = run_git(&remote_path, &["rev-parse", "refs/heads/main"]);

    assert_eq!(persisted.status, CoordinationTaskStatus::Completed);
    assert_eq!(local_head, remote_head);
}
