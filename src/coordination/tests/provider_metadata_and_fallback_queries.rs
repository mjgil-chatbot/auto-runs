// ── Serde compatibility, provider attribution, and fallback query tests ──

#[test]
fn coordination_event_deserializes_legacy_without_provider() {
    let legacy = r#"{
        "at": "2026-04-05T12:00:00Z",
        "event_type": "task_claimed",
        "task_id": "task-1",
        "message": "Started: survey workspace"
    }"#;

    let event: CoordinationEvent =
        serde_json::from_str(legacy).expect("legacy event should deserialize");
    assert_eq!(event.event_type, "task_claimed");
    assert_eq!(event.task_id.as_deref(), Some("task-1"));
    assert_eq!(event.provider, None);
}

#[test]
fn coordination_event_round_trips_with_provider() {
    let event = CoordinationEvent {
        at: DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        event_type: "task_completed".to_string(),
        task_id: Some("task-1".to_string()),
        provider: Some("codex".to_string()),
        category: None,
        fallback_from: None,
        message: "Completed: implement schema".to_string(),
    };

    let json = serde_json::to_string(&event).expect("event should serialize");
    assert!(json.contains("\"provider\":\"codex\""));

    let decoded: CoordinationEvent = serde_json::from_str(&json).expect("event should deserialize");
    assert_eq!(decoded.provider.as_deref(), Some("codex"));
    assert_eq!(decoded.event_type, "task_completed");
}

#[test]
fn task_artifact_deserializes_legacy_without_provider_metadata() {
    let legacy = r#"{
        "run_id": "run-1",
        "task_id": "task-1",
        "summary": "done",
        "body": "details",
        "completed_at": "2026-04-05T12:00:00Z",
        "report": {
            "outcome": "completed_with_changes",
            "summary": "done",
            "details": "details",
            "changed_files": ["src/main.rs"],
            "tests_run": [],
            "warnings": [],
            "blockers": [],
            "remaining_gaps": [],
            "design_requirements_satisfied": [],
            "validation_errors": [],
            "fix_tasks": []
        }
    }"#;

    let artifact: TaskArtifact =
        serde_json::from_str(legacy).expect("legacy artifact should deserialize");
    assert_eq!(artifact.run_id, "run-1");
    assert_eq!(artifact.provider, None);
    assert_eq!(artifact.task_category, None);
    assert_eq!(artifact.fallback_from, None);
    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
}

#[test]
fn task_artifact_round_trips_with_provider_metadata() {
    let artifact = TaskArtifact {
        run_id: "run-1".to_string(),
        task_id: "task-1".to_string(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: Some("claude".to_string()),
        summary: "implemented schema".to_string(),
        body: "implementation details".to_string(),
        completed_at: DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "implemented schema".to_string(),
            details: "details".to_string(),
            changed_files: vec!["src/storage/mod.rs".to_string()],
            ..Default::default()
        },
    };

    let json = serde_json::to_string(&artifact).expect("artifact should serialize");
    assert!(json.contains("\"provider\":\"codex\""));
    assert!(json.contains("\"task_category\":\"deep_write\""));
    assert!(json.contains("\"fallback_from\":\"claude\""));

    let decoded: TaskArtifact = serde_json::from_str(&json).expect("artifact should deserialize");
    assert_eq!(decoded.provider.as_deref(), Some("codex"));
    assert_eq!(decoded.task_category, Some(TaskCategory::DeepWrite));
    assert_eq!(decoded.fallback_from.as_deref(), Some("claude"));
}

#[test]
fn task_artifact_omits_none_provider_fields_in_json() {
    let artifact = TaskArtifact {
        run_id: "run-1".to_string(),
        task_id: "task-1".to_string(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "done".to_string(),
        body: "details".to_string(),
        completed_at: DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        report: TaskExecutionReport::default(),
    };

    let json = serde_json::to_string(&artifact).expect("artifact should serialize");
    // None fields should be omitted from JSON (skip_serializing_if)
    assert!(!json.contains("\"provider\""));
    assert!(!json.contains("\"task_category\""));
    assert!(!json.contains("\"fallback_from\""));
}

#[test]
fn append_event_with_provider_persists_provider_field() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let tasks = vec![fixture.make_task(
        &run,
        "task-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write module",
        &["src"],
        &["src/mod.rs"],
        &[],
        CoordinationTaskStatus::Running,
    )];
    fixture.persist_run_with_tasks(&run, &tasks);

    append_event_with_provider(
        &run.run_id,
        "task_completed",
        Some("task-1"),
        Some("codex"),
        None,
        None,
        "Completed write module",
    )
    .expect("append_event_with_provider should succeed");

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(!events.is_empty());
    let last_event = events.last().unwrap();
    assert_eq!(last_event.event_type, "task_completed");
    assert_eq!(last_event.provider.as_deref(), Some("codex"));
    assert_eq!(last_event.task_id.as_deref(), Some("task-1"));
}

#[test]
fn append_event_without_provider_leaves_provider_none() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    fixture.persist_run_with_tasks(&run, &[]);

    append_event(&run.run_id, "run_created", None, "Created run")
        .expect("append_event should succeed");

    let events = load_events(&run.run_id, None, None).expect("events should load");
    assert!(!events.is_empty());
    let last_event = events.last().unwrap();
    assert_eq!(last_event.event_type, "run_created");
    assert_eq!(last_event.provider, None);
}

#[test]
fn coordination_event_provider_field_survives_jsonl_round_trip() {
    let event = CoordinationEvent {
        at: Utc::now(),
        event_type: "task_claimed".to_string(),
        task_id: Some("task-1".to_string()),
        provider: Some("gemini".to_string()),
        category: None,
        fallback_from: None,
        message: "Started task via gemini".to_string(),
    };

    // Simulate JSONL round-trip (single line)
    let json_line = serde_json::to_string(&event).unwrap();
    assert!(!json_line.contains('\n'));
    let decoded: CoordinationEvent = serde_json::from_str(&json_line).unwrap();
    assert_eq!(decoded.provider.as_deref(), Some("gemini"));
}

#[test]
fn task_artifact_fallback_provenance_tracks_rerouted_provider() {
    // Simulate a task that was intended for claude but rerouted to codex
    let artifact = TaskArtifact {
        run_id: "run-1".to_string(),
        task_id: "task-1".to_string(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        fallback_from: Some("claude".to_string()),
        summary: "Completed via fallback".to_string(),
        body: "details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            ..Default::default()
        },
    };

    let json = serde_json::to_string(&artifact).unwrap();
    let decoded: TaskArtifact = serde_json::from_str(&json).unwrap();

    // Provider records who actually ran the task
    assert_eq!(decoded.provider.as_deref(), Some("codex"));
    // fallback_from records the original intended provider
    assert_eq!(decoded.fallback_from.as_deref(), Some("claude"));
    // task_category records what kind of work this was
    assert_eq!(decoded.task_category, Some(TaskCategory::DeepWrite));
}

#[test]
fn query_runs_filtered_falls_back_to_authoritative_files_without_index_db() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    fixture.persist_run_with_tasks(&run, &[]);

    assert!(
        !crate::storage::index_db_path().exists(),
        "test should start without an SQLite index"
    );

    let runs = query_runs_filtered(
        Some(run.status.as_str()),
        Some(run.provider.as_str()),
        Some(run.workflow_mode.as_str()),
        Some(10),
        None,
    )
    .expect("fallback run query should succeed");

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, run.run_id);
    assert_eq!(runs[0].provider, run.provider);
    assert_eq!(
        runs[0].workflow_mode.as_deref(),
        Some(run.workflow_mode.as_str())
    );
}

#[test]
fn query_runs_filtered_skips_non_run_directories_during_authoritative_fallback() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    fixture.persist_run_with_tasks(&run, &[]);

    let root = coordination_root_dir();
    fs::create_dir_all(root.join("debug-runsc-real-codex")).unwrap();
    fs::create_dir_all(root.join("review-plan-repair-bogus")).unwrap();
    fs::write(root.join("background-driver.json"), "{}").unwrap();

    let runs = query_runs_filtered(None, None, None, None, None)
        .expect("fallback run query should ignore non-run directories");

    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].run_id, run.run_id);
}

#[test]
fn query_events_falls_back_to_authoritative_files_without_index_db() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let tasks = vec![fixture.make_task(
        &run,
        "task-1",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write module",
        &["src"],
        &["src/mod.rs"],
        &[],
        CoordinationTaskStatus::Running,
    )];
    fixture.persist_run_with_tasks(&run, &tasks);

    let event = CoordinationEvent {
        at: Utc::now(),
        event_type: "task_completed".to_string(),
        task_id: Some("task-1".to_string()),
        provider: None,
        category: None,
        fallback_from: None,
        message: "Completed write module".to_string(),
    };
    fs::write(
        events_path(&run.run_id),
        format!("{}\n", serde_json::to_string(&event).unwrap()),
    )
    .expect("event log should persist");

    assert!(
        !crate::storage::index_db_path().exists(),
        "test should start without an SQLite index"
    );

    let events = query_events(
        Some(&run.run_id),
        Some("task_completed"),
        Some("codex"),
        None,
        None,
        None,
    )
    .expect("fallback event query should succeed");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].run_id, run.run_id);
    assert_eq!(events[0].task_id.as_deref(), Some("task-1"));
    assert_eq!(events[0].provider.as_deref(), Some("codex"));
}
