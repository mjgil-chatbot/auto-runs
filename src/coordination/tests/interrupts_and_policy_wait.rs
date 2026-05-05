fn make_manual_policy_interrupt(
    run_id: &str,
    task_id: Option<&str>,
    title: &str,
) -> CoordinationInterrupt {
    let interrupt = CoordinationInterrupt {
        interrupt_id: Uuid::new_v4().to_string(),
        run_id: run_id.to_string(),
        task_id: task_id.map(str::to_string),
        council_entry_id: None,
        kind: InterruptKind::ManualPolicyDecision,
        status: InterruptStatus::Open,
        title: title.to_string(),
        prompt: "Need operator approval before work continues.".to_string(),
        payload: serde_json::json!({ "reason": "manual policy pause" }),
        created_at: Utc::now(),
        resolved_at: None,
        resolution: None,
    };
    create_interrupt(&interrupt).expect("interrupt should persist");
    interrupt
}

#[test]
fn open_interrupt_blocks_claims_and_drive_until_resolution() {
    let fixture = CoordinationTestFixture::named("interrupt-pause");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "survey",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));
    let interrupt =
        make_manual_policy_interrupt(&run.run_id, Some(&task.task_id), "Awaiting approval");

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");
    assert_eq!(snapshot.run.status, CoordinationRunStatus::WaitingForPolicy);
    assert!(claim_next_runnable_task(&run.run_id, 71, "codex")
        .expect("claim should check interrupt")
        .is_none());
    assert!(
        !drive_run_for_instance(&run.run_id, 71, "codex").expect("driver should honor pause")
    );
    assert_eq!(
        load_task(&run.run_id, &task.task_id)
            .expect("task should reload")
            .status,
        CoordinationTaskStatus::Queued
    );

    resolve_interrupt(
        &run.run_id,
        &interrupt.interrupt_id,
        InterruptResolution::Approved {
            note: "resume".to_string(),
        },
    )
    .expect("interrupt should resolve");

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should refresh");
    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
    assert!(drive_run_for_instance(&run.run_id, 71, "codex").expect("driver should resume"));
    assert_eq!(
        load_task(&run.run_id, &task.task_id)
            .expect("task should reload")
            .status,
        CoordinationTaskStatus::Completed
    );

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events
        .iter()
        .any(|event| event.event_type == "interrupt_opened"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "interrupt_resolved"));
}

#[test]
fn rejecting_review_approval_interrupt_fails_apply_tasks() {
    let fixture = CoordinationTestFixture::named("review-approval-interrupt");
    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    run.require_review_before_apply = true;
    run.auto_retry_failed_tasks = false;
    let review = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review generated work",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let apply = fixture.make_task(
        &run,
        "apply",
        CoordinationTaskKind::ApplyApprovedEdits,
        Some("codex"),
        "Apply reviewed edits",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[&review.task_id],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[review.clone(), apply.clone()]);

    let interrupt = create_review_approval_interrupt(
        &run.run_id,
        &review.task_id,
        "Review approval needed",
        "Approve these edits before apply runs.",
        serde_json::json!({
            "summary": "one pending apply step",
            "fix_tasks": [],
        }),
    )
    .expect("review interrupt should persist");

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should load");
    assert_eq!(snapshot.run.status, CoordinationRunStatus::WaitingForPolicy);

    resolve_interrupt(
        &run.run_id,
        &interrupt.interrupt_id,
        InterruptResolution::Rejected {
            note: "needs manual follow-up".to_string(),
        },
    )
    .expect("interrupt should resolve");

    let apply = load_task(&run.run_id, &apply.task_id).expect("apply task should reload");
    assert_eq!(apply.status, CoordinationTaskStatus::Failed);
    assert_eq!(apply.claimed_by, None);
    
    assert_eq!(
        apply.artifact_summary.as_deref(),
        Some("Review approval rejected: needs manual follow-up")
    );
    assert!(!has_open_interrupts(&run.run_id));

    let snapshot = load_run_snapshot(&run.run_id).expect("snapshot should refresh");
    assert_eq!(snapshot.run.status, CoordinationRunStatus::Failed);
    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(events.iter().any(|event| {
        event.event_type == "task_failed" && event.task_id.as_deref() == Some(apply.task_id.as_str())
    }));
}
