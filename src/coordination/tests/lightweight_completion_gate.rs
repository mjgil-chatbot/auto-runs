#[test]
fn lightweight_completion_gate_ignores_superseded_partial_reviews() {
    let fixture = CoordinationTestFixture::named("lightweight-completion-gate");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let initial_review = fixture.make_task(
        &run,
        "review-1",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate lightweight implementation pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[
            "docs/autonomous/lightweight-tasks.md",
            "docs/autonomous/lightweight-tasks-pass-1.md",
        ],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let follow_up_review = fixture.make_task_with_options(
        &run,
        "review-2",
        Some("review-1"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec![
            "artifacts/**".to_string(),
            "**".to_string(),
            "docs/design.md".to_string(),
        ],
        vec![
            "docs/autonomous/lightweight-tasks-pass-1.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-2.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[initial_review.clone(), follow_up_review.clone()]);

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: initial_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "more work remains".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "more work remains".to_string(),
            details: "queued a follow-up pass".to_string(),
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Finish remaining slice".to_string(),
                provider: None,
                prompt: "Finish the remaining implementation slice.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["src".to_string()],
                write_scope: vec!["src/lib.rs".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec!["remaining slice is implemented".to_string()],
            }],
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: follow_up_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "validation passed".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "validation passed".to_string(),
            details: "no further follow-up is required".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
}

#[test]
fn completion_gate_ignores_superseded_partial_planning_tasks() {
    let fixture = CoordinationTestFixture::named("planner-completion-gate");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "main", 5);
    let validation_plan = fixture.make_task_with_options(
        &run,
        "validation-plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split GitHub branch fix validation tasks",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let generated_validation = fixture.make_task_with_options(
        &run,
        "validation-read",
        Some("validation-plan"),
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Verify GitHub Actions workflow coverage for main",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec!["validation-plan".to_string()],
        CoordinationTaskStatus::Completed,
    );
    let review = fixture.make_task_with_options(
        &run,
        "review",
        Some("validation-plan"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate GitHub branch fix pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec!["validation-read".to_string()],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(
        &run,
        &[
            validation_plan.clone(),
            generated_validation.clone(),
            review.clone(),
        ],
    );

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: validation_plan.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Verify),
        fallback_from: None,
        summary: "queued validation work".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "queued validation work".to_string(),
            details: "delegated the branch-fix validation checks".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: generated_validation.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Verify),
        fallback_from: None,
        summary: "workflow coverage verified".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "workflow coverage verified".to_string(),
            details: "the branch has workflow coverage".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "branch fix validated".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "branch fix validated".to_string(),
            details: "no further follow-up is required".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
}

#[test]
fn completion_gate_ignores_superseded_partial_branch_fix_write_tasks() {
    let fixture = CoordinationTestFixture::named("branch-fix-write-completion-gate");
    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.notes = github_branch_fix_notes_with_settings("", "build", 5);
    let write_task = fixture.make_task(
        &run,
        "write-fix",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Add test coverage to most recent fix commit",
        &["tests", "src"],
        &["tests/core_test.rs", "tests/core_test.sh"],
        &["validation review depends on this write"],
        CoordinationTaskStatus::Completed,
    );
    let review_task = fixture.make_task_with_options(
        &run,
        "review",
        Some("validation-plan"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate GitHub branch fix pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec![write_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[write_task.clone(), review_task.clone()]);

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: write_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "pushed fix, waiting on CI".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "pushed fix, waiting on CI".to_string(),
            details: "latest branch run is still in progress".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "branch fix validated".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "branch fix validated".to_string(),
            details: "successful CI was verified in the terminal review".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
}

#[test]
fn lightweight_completion_gate_ignores_publish_partial_after_completed_repair_task() {
    let fixture = CoordinationTestFixture::named("lightweight-publish-repair-completion-gate");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let write_task = fixture.make_task(
        &run,
        "write-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Strengthen Tests For Supported And Rejected Graphs",
        &["src", "tests"],
        &["src/lib.rs", "tests/core.rs"],
        &["write task depends on implementation slice"],
        CoordinationTaskStatus::Completed,
    );
    let final_review = fixture.make_task_with_options(
        &run,
        "review-final",
        Some("plan-1"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Follow-up review after Follow-up review after Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec![write_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    let repair_task = fixture.make_task(
        &run,
        "repair-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Repair repository after failed run",
        &["**"],
        &["src/lib.rs"],
        &["repair depends on publish failure"],
        CoordinationTaskStatus::Completed,
    );
    let mut repair_task = repair_task;
    repair_task.dependencies = vec![write_task.task_id.clone(), final_review.task_id.clone()];
    fixture.persist_run_with_tasks(
        &run,
        &[
            write_task.clone(),
            final_review.clone(),
            repair_task.clone(),
        ],
    );

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: write_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "publish failed after successful write".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "publish failed after successful write".to_string(),
            details: "the implementation was correct but automatic git publish hit a remote race"
                .to_string(),
            ..Default::default()
        },
    });
    let mut persisted_write =
        load_task(&run.run_id, &write_task.task_id).expect("write task should load");
    persisted_write.artifact_summary =
        Some("Deferred git publish failed: git push failed: remote race".to_string());
    fixture.persist_run_with_tasks(
        &run,
        &[
            persisted_write.clone(),
            final_review.clone(),
            repair_task.clone(),
        ],
    );

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: final_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "validation passed".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "validation passed".to_string(),
            details: "the latest lightweight pass is approved".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: repair_task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "repository repaired and pushed".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "repository repaired and pushed".to_string(),
            details: "the repair task resolved the deferred publish blocker".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
}

#[test]
fn lightweight_completion_gate_keeps_non_publish_partial_tasks_failed() {
    let fixture = CoordinationTestFixture::named("lightweight-non-publish-partial-fails");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    // Isolate completion gating from optional whole-run self-healing.
    run.auto_retry_failed_tasks = false;
    let validation_task = fixture.make_task(
        &run,
        "validation-read",
        CoordinationTaskKind::GeneratedReadTask,
        Some("codex"),
        "Validate Runner File Discovery And Repair Safety",
        &["src", "tests"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let repair_task = fixture.make_task(
        &run,
        "repair-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Repair repository after failed run",
        &["**"],
        &["src/lib.rs"],
        &["repair depends on prior validation"],
        CoordinationTaskStatus::Completed,
    );
    let mut repair_task = repair_task;
    repair_task.dependencies = vec![validation_task.task_id.clone()];
    fixture.persist_run_with_tasks(&run, &[validation_task.clone(), repair_task.clone()]);

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: validation_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Verify),
        fallback_from: None,
        summary: "coverage is still incomplete".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "coverage is still incomplete".to_string(),
            details: "required direct guard tests are still missing".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: repair_task.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "repo repaired".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "repo repaired".to_string(),
            details: "workspace state is healthy but the validation gap remains".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Failed);
}

#[test]
fn lightweight_completion_gate_ignores_partial_write_closed_by_descendant_review() {
    let fixture =
        CoordinationTestFixture::named("lightweight-partial-write-closed-by-descendant-review");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let partial_write = fixture.make_task(
        &run,
        "write-partial",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Harden MVP Regression Coverage",
        &["src", "tests"],
        &["src/lib.rs", "tests/cli.rs"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let partial_review = fixture.make_task_with_options(
        &run,
        "review-partial",
        Some("parent-review"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec![partial_write.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    let mut completed_review = fixture.make_task_with_options(
        &run,
        "review-final",
        Some("review-partial"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Follow-up review after Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    completed_review.artifact_summary =
        Some("current pass objectives are satisfied [completed_no_change]".to_string());
    fixture.persist_run_with_tasks(
        &run,
        &[
            partial_write.clone(),
            partial_review.clone(),
            completed_review.clone(),
        ],
    );

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: partial_write.task_id.clone(),
        provider: Some("minimax".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: None,
        summary: "combat-event coverage still depends on a later fix".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "combat-event coverage still depends on a later fix".to_string(),
            details: "the regression gate was added before the combat fix landed".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: partial_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "a cleanup pass is still required".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "a cleanup pass is still required".to_string(),
            details: "queued a final follow-up review".to_string(),
            ..Default::default()
        },
    });
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: completed_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "current pass objectives are satisfied".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "current pass objectives are satisfied".to_string(),
            details: "the final review closed the remaining gaps".to_string(),
            ..Default::default()
        },
    });

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Completed);
}

#[test]
fn lightweight_review_non_actionable_fix_tasks_do_not_spawn_follow_up_review() {
    let fixture = CoordinationTestFixture::named("lightweight-non-actionable-review-stop");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate lightweight implementation pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[
            "docs/design.md",
            "docs/autonomous/lightweight-tasks.md",
            "docs/autonomous/lightweight-tasks-pass-1.md",
        ],
        &[],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[review_task.clone()]);

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "needs external validation environment".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "needs external validation environment".to_string(),
            details: "the next step only reruns proof after the external environment is fixed"
                .to_string(),
            remaining_gaps: vec!["zig is missing from the validation environment".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Re-run proof when environment is fixed".to_string(),
                provider: None,
                prompt:
                    "Re-run the canonical proof after the external Zig environment is corrected."
                        .to_string(),
                kind: "read".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: Vec::new(),
                allowed_tools: vec!["filesystem".to_string(), "search".to_string()],
                acceptance_checks: vec!["the canonical proof is rerun".to_string()],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    assert_eq!(tasks.len(), 1);
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "review_recursion_suppressed"
            && event
                .message
                .contains("non-actionable repo-local next steps")
    }));
}

#[test]
fn lightweight_review_duplicate_blocker_signature_does_not_spawn_follow_up_review() {
    let fixture = CoordinationTestFixture::named("lightweight-duplicate-blocker-stop");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let parent_review = fixture.make_task(
        &run,
        "review-1",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Validate lightweight implementation pass",
        &["artifacts/**", "**", "docs/design.md"],
        &[
            "docs/design.md",
            "docs/autonomous/lightweight-tasks.md",
            "docs/autonomous/lightweight-tasks-pass-1.md",
        ],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let child_review = fixture.make_task_with_options(
        &run,
        "review-2",
        Some("review-1"),
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Follow-up review after Validate lightweight implementation pass",
        vec!["filesystem".to_string(), "search".to_string()],
        vec![
            "artifacts/**".to_string(),
            "**".to_string(),
            "docs/design.md".to_string(),
        ],
        vec![
            "docs/design.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-1.md".to_string(),
            "docs/autonomous/lightweight-tasks-pass-2.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[parent_review.clone(), child_review.clone()]);

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: parent_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "zig missing from validation environment".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "zig missing from validation environment".to_string(),
            details: "the canonical proof cannot run because zig is missing from PATH".to_string(),
            blockers: vec!["zig missing from validation environment".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Repair local docs".to_string(),
                provider: None,
                prompt: "Repair a local repo issue.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: vec!["**".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "a local fix is applied".to_string(),
                    "tests cover the fix".to_string(),
                ],
            }],
            ..Default::default()
        },
    });

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: child_review.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "zig missing from validation environment".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "zig missing from validation environment".to_string(),
            details: "the canonical proof cannot run because zig is missing from PATH".to_string(),
            blockers: vec!["zig missing from validation environment".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Repair local docs again".to_string(),
                provider: None,
                prompt: "Try another local repo fix.".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["**".to_string()],
                write_scope: vec!["**".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "a follow-up local fix is applied".to_string(),
                    "tests cover the follow-up fix".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &child_review, &artifact).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    assert_eq!(tasks.len(), 2);
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "review_recursion_suppressed"
            && event
                .message
                .contains("repeated the same blocker signature")
    }));
}
