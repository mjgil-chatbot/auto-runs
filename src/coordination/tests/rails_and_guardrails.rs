#[test]
fn execute_provider_task_input_rail_rejects_missing_design_doc_and_logs_event() {
    let fixture = CoordinationTestFixture::named("rails-input");
    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    run.auto_retry_failed_tasks = false;
    let task = fixture.make_task(
        &run,
        "plan-task",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan implementation tasks",
        &["docs/design.md", "src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let err = execute_provider_task(&task, "codex", 77, Arc::new(AtomicBool::new(false)))
        .expect_err("missing design doc should reject before provider execution");

    assert!(err
        .to_string()
        .contains("input rail rejected task: required canonical design doc"));
    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "rail_decision"
            && event.task_id.as_deref() == Some(task.task_id.as_str())
            && event.message.contains("Input rail: reject")
    }));
}

#[test]
fn lightweight_bootstrap_plan_allows_missing_design_doc_before_first_split() {
    let fixture = CoordinationTestFixture::named("rails-lightweight-bootstrap");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["codex".to_string()],
    );
    let mut task = fixture.make_task_with_options(
        &run,
        "plan-task",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split lightweight implementation tasks",
        vec![
            "filesystem".to_string(),
            "search".to_string(),
            "edit".to_string(),
        ],
        vec!["**".to_string()],
        vec![
            "docs/design.md".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    task.task_category = Some(TaskCategory::Plan);

    assert!(matches!(check_input_rail(&run, &task), RailDecision::Allow));
}

#[test]
fn check_dialog_rail_rewrites_missing_contract_scaffold_and_scope_reminder() {
    let fixture = CoordinationTestFixture::named("rails-dialog");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "write-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let decision = check_dialog_rail(&task, &task.prompt);
    let RailDecision::Rewrite { replacement } = decision else {
        panic!("dialog rail should rewrite missing scaffold");
    };
    assert!(replacement.contains("Return JSON only"));
    assert!(replacement.contains("validate_autonomous_task_report.py"));
    assert!(replacement.contains("You may modify files only inside this declared write scope"));
}

#[test]
fn check_execution_rail_rejects_sync_back_without_edit_tool() {
    let fixture = CoordinationTestFixture::named("rails-execution-tool");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        vec!["filesystem".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );

    let decision = check_execution_rail(
        &run,
        &task,
        &[MaterializedArtifactFile {
            path: "src/lib.rs".to_string(),
            content: "pub fn sample() {}\n".to_string(),
        }],
    );

    let RailDecision::Reject { reason } = decision else {
        panic!("execution rail should reject missing edit tool");
    };
    assert!(reason.contains("without declaring the edit tool"));
}

#[test]
fn complete_task_execution_rail_rejects_apply_without_review_approval_and_logs_event() {
    let fixture = CoordinationTestFixture::named("rails-execution-approval");
    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    run.require_review_before_apply = true;
    run.auto_retry_failed_tasks = false;
    let review_task = fixture.make_task(
        &run,
        "review-task",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review artifacts",
        &["artifacts/**", "docs/design.md"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let mut apply_task = fixture.make_task(
        &run,
        "apply-task",
        CoordinationTaskKind::ApplyApprovedEdits,
        Some("codex"),
        "Apply approved edits",
        &["**"],
        &["."],
        &["review-task"],
        CoordinationTaskStatus::Running,
    );
    apply_task.claimed_by = Some(88);
    fixture.persist_run_with_tasks(&run, &[review_task, apply_task.clone()]);
    assert!(create_claim_file(&make_claim_for_test(
        &run.run_id,
        &apply_task.task_id,
        88,
        88
    ))
    .expect("claim should persist"));
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { println!(\"updated\"); }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &apply_task.task_id,
        88,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: apply_task.task_id.clone(),
            provider: Some("codex".to_string()),
            task_category: None,
            fallback_from: None,
            summary: "Applied the approved edits".to_string(),
            body: "Updated src/lib.rs".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedWithChanges,
                summary: "Applied the approved edits".to_string(),
                details: "Updated src/lib.rs".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                ..Default::default()
            },
        },
    )
    .expect("completion should finalize into validation failure");

    let persisted = load_task(&run.run_id, &apply_task.task_id).expect("task should reload");
    assert_eq!(persisted.status, CoordinationTaskStatus::Failed);
    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "rail_decision"
            && event.task_id.as_deref() == Some(apply_task.task_id.as_str())
            && event.message.contains("Execution rail: reject")
    }));
}

#[test]
fn complete_task_output_rail_rejects_stub_marker_sync_back_and_logs_event() {
    let fixture = CoordinationTestFixture::named("rails-output");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.auto_retry_failed_tasks = false;
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
    task.claimed_by = Some(99);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(
        create_claim_file(&make_claim_for_test(&run.run_id, &task.task_id, 99, 99))
            .expect("claim should persist")
    );
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "pub fn sample() { todo!() }\n",
    )
    .unwrap();

    complete_task(
        &run.run_id,
        &task.task_id,
        99,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: Some("codex".to_string()),
            task_category: None,
            fallback_from: None,
            summary: "Updated src/lib.rs".to_string(),
            body: "Updated src/lib.rs".to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedWithChanges,
                summary: "Updated src/lib.rs".to_string(),
                details: "Updated src/lib.rs".to_string(),
                changed_files: vec!["src/lib.rs".to_string()],
                ..Default::default()
            },
        },
    )
    .expect("completion should finalize into validation failure");

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "rail_decision"
            && event.task_id.as_deref() == Some(task.task_id.as_str())
            && event.message.contains("Output rail: reject")
    }));
}
