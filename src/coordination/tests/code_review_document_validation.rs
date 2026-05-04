#[test]
fn code_review_materialized_doc_allows_stub_marker_discussion_in_prose() {
    let fixture = CoordinationTestFixture::named("code-review-doc-stub-prose");
    let run = fixture.make_run(AutonomousWorkflowMode::CodeReview);
    let task = fixture.make_task(
        &run,
        "materialize-review",
        CoordinationTaskKind::MaterializeDesignDoc,
        Some("codex"),
        "Write code review document",
        &["artifacts/**"],
        &["docs/code-review.md"],
        &[],
        CoordinationTaskStatus::Running,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let output_path = fixture.workspace.path().join("docs/code-review.md");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    let body = "# Code Review\n\nThe review can mention `todo!` or `not yet implemented` when describing findings.\n";
    fs::write(&output_path, body).unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Review),
        fallback_from: None,
        summary: "Materialized review".to_string(),
        body: body.to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Materialized review".to_string(),
            details: "Wrote docs/code-review.md".to_string(),
            changed_files: vec!["docs/code-review.md".to_string()],
            ..Default::default()
        },
    };

    let errors = validate_task_artifact(&run.run_id, &task, &artifact).unwrap();
    assert!(!errors
        .iter()
        .any(|error| error.contains("still contains blocked stub markers")));
}
