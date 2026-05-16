#[test]
fn file_contains_stub_markers_ignores_rust_string_literal_fixtures() {
    let fixture = CoordinationTestFixture::named("stub-markers-string-literal");
    let file_path = fixture.workspace.path().join("src/test_fixture.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(
        &file_path,
        "const SAMPLE: &str = \"pub fn sample() { unimplemented!() }\";\nfn helper() {}\n",
    )
    .unwrap();

    assert!(
        !file_contains_stub_markers(fixture.workspace.path(), "src/test_fixture.rs").unwrap()
    );
}

#[test]
fn file_contains_stub_markers_ignores_rust_raw_string_literals() {
    let fixture = CoordinationTestFixture::named("stub-markers-raw-string");
    let file_path = fixture.workspace.path().join("src/raw_fixture.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(
        &file_path,
        "const SAMPLE: &str = r#\"todo!() and not yet implemented\"#;\nfn helper() {}\n",
    )
    .unwrap();

    assert!(!file_contains_stub_markers(fixture.workspace.path(), "src/raw_fixture.rs").unwrap());
}

#[test]
fn validate_task_artifact_allows_rust_test_fixture_stub_literals() {
    let fixture = CoordinationTestFixture::named("artifact-stub-fixture-string");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "write-test-fixture",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Update test fixture",
        &["src/test_fixture.rs"],
        &["src/test_fixture.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );

    let output_path = fixture.workspace.path().join("src/test_fixture.rs");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::write(
        &output_path,
        "const SAMPLE: &str = \"assert!(contains_stub_markers(\\\"todo!\\\"))\";\nfn helper() {}\n",
    )
    .unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Updated test fixture".to_string(),
        body: "const SAMPLE: &str = \"assert!(contains_stub_markers(\\\"todo!\\\"))\";\nfn helper() {}\n"
            .to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Updated test fixture".to_string(),
            details: "Refined the fixture text".to_string(),
            changed_files: vec!["src/test_fixture.rs".to_string()],
            ..Default::default()
        },
    };

    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    let errors = validate_task_artifact(&run.run_id, &task, &artifact).unwrap();
    assert!(!errors
        .iter()
        .any(|error| error.contains("still contains blocked stub markers")));
}
