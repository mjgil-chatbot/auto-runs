#[test]
fn build_provider_task_artifact_for_ralph_verify_uses_default_reason() {
    let fixture = CoordinationTestFixture::named("artifact-ralph-verify");
    let run = fixture.make_run(AutonomousWorkflowMode::RalphLoop);
    let task = fixture.make_task(
        &run,
        "verify",
        CoordinationTaskKind::RalphVerify,
        Some("codex"),
        "Verify",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(&task, r#"{"complete":false,"reason":"   "}"#)
        .expect("ralph verify artifact should build");

    assert_eq!(artifact.report.outcome, CoordinationTaskOutcome::Partial);
    assert_eq!(artifact.summary, "Completion criteria not yet satisfied.");
    assert_eq!(
        artifact.report.remaining_gaps,
        vec!["Completion criteria not yet satisfied.".to_string()]
    );
}

#[test]
fn build_provider_task_artifact_for_ralph_execute_normalizes_changed_files() {
    let fixture = CoordinationTestFixture::named("artifact-ralph-execute");
    let run = fixture.make_run(AutonomousWorkflowMode::RalphLoop);
    let task = fixture.make_task(
        &run,
        "execute",
        CoordinationTaskKind::RalphExecute,
        Some("codex"),
        "Execute",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(
        &task,
        r#"{"outcome":"completed_with_changes","summary":"   ","reason":"did the work","changed_files":[" `src/lib.rs` ","  "]}"#,
    )
    .expect("ralph execute artifact should build");

    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert_eq!(artifact.summary, "Made changes to workspace");
    assert_eq!(
        artifact.report.changed_files,
        vec!["src/lib.rs".to_string()]
    );
}

#[test]
fn build_provider_task_artifact_uses_task_title_and_raw_response_fallbacks() {
    let fixture = CoordinationTestFixture::named("artifact-standard");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "writer",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(
        &task,
        r#"{"outcome":"completed_no_change","summary":"   ","details":"   ","changed_files":[]}"#,
    )
    .expect("standard artifact should build");

    assert_eq!(artifact.summary, "Write task");
    assert!(artifact
        .body
        .contains("\"outcome\":\"completed_no_change\""));
}

#[test]
fn build_provider_task_artifact_normalizes_malformed_standard_report() {
    let fixture = CoordinationTestFixture::named("artifact-standard-malformed");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "writer",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("gemini"),
        "Write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let artifact =
        build_provider_task_artifact(&task, "summary: fixed it\nchanged_files: [src/lib.rs]")
            .expect("malformed standard artifact should still build");

    assert_eq!(artifact.report.outcome, CoordinationTaskOutcome::Failed);
    assert_eq!(
        artifact.summary,
        "Provider returned a malformed task report."
    );
    assert!(artifact
        .body
        .contains("Rust normalized a malformed provider task report"));
    assert!(artifact.body.contains("Raw provider response:"));
    assert!(artifact.body.contains("summary: fixed it"));
    assert_eq!(
        artifact.report.warnings,
        vec!["Recovered malformed provider task output into a failure report.".to_string()]
    );
}

#[test]
fn build_provider_task_artifact_for_lightweight_generated_write_accepts_plaintext_status() {
    let fixture = CoordinationTestFixture::named("artifact-lightweight-plaintext-write");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "writer",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Create static viewer shell",
        &["docs/design.md"],
        &["index.html", "src/styles.css"],
        &["shell renders", "styles load"],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(
        &task,
        "COMPLETED: Created the static viewer shell.\nImplemented the requested layout and satisfied shell renders plus styles load.",
    )
    .expect("lightweight plaintext artifact should build");

    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert_eq!(artifact.summary, "Created the static viewer shell.");
    assert!(artifact.report.changed_files.is_empty());
    assert!(artifact.body.contains("Implemented the requested layout"));
}

#[test]
fn build_provider_task_artifact_for_lightweight_generated_write_accepts_loose_json() {
    let fixture = CoordinationTestFixture::named("artifact-lightweight-loose-json");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "writer",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Create static viewer shell",
        &["docs/design.md"],
        &["index.html", "src/styles.css"],
        &["shell renders", "styles load"],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(
        &task,
        r#"{
            "outcome": "completed_with_changes",
            "summary": "Created the static viewer shell",
            "details": "Implemented the HTML shell and stylesheet scaffolding.",
            "tests_run": false,
            "warnings": [],
            "blockers": [],
            "remaining_gaps": [],
            "design_requirements_satisfied": ["shell renders", "styles load"]
        }"#,
    )
    .expect("lightweight loose-json artifact should build");

    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert_eq!(artifact.summary, "Created the static viewer shell");
    assert_eq!(
        artifact.report.design_requirements_satisfied,
        vec!["shell renders".to_string(), "styles load".to_string()]
    );
    assert!(artifact.report.changed_files.is_empty());
}

#[test]
fn parse_task_execution_report_normalizes_fix_task_checks_and_downgrades_empty_changes() {
    let report = parse_task_execution_report(
        r#"{
            "outcome":"completed_with_changes",
            "summary":"  summary  ",
            "details":"  details  ",
            "changed_files":["   ",""],
            "tests_run":[" cargo test "," "],
            "warnings":[" warning "],
            "blockers":[],
            "remaining_gaps":[],
            "design_requirements_satisfied":[" requirement "],
            "validation_errors":[" validation "],
            "fix_tasks":[
                {
                    "title":"Patch src/lib.rs",
                    "provider":"codex",
                    "prompt":"Patch it",
                    "kind":"write",
                    "read_scope":["src/lib.rs"],
                    "write_scope":["src/lib.rs"],
                    "allowed_tools":["filesystem","edit"],
                    "acceptance_checks":[" check one "," ","check two "]
                }
            ]
        }"#,
    )
    .expect("report should parse");

    assert_eq!(report.outcome, CoordinationTaskOutcome::CompletedNoChange);
    assert_eq!(report.summary, "summary");
    assert_eq!(report.details, "details");
    assert_eq!(report.tests_run, vec!["cargo test".to_string()]);
    assert_eq!(report.warnings, vec!["warning".to_string()]);
    assert_eq!(
        report.design_requirements_satisfied,
        vec!["requirement".to_string()]
    );
    assert_eq!(report.validation_errors, vec!["validation".to_string()]);
    assert_eq!(
        report.fix_tasks[0].acceptance_checks,
        vec!["check one".to_string(), "check two".to_string()]
    );
}

#[test]
fn parse_task_execution_report_rejects_non_object_json() {
    let err = parse_task_execution_report(r#"[{"outcome":"completed_no_change"}]"#)
        .expect_err("non-object JSON should fail");
    assert!(err
        .to_string()
        .contains("missing JSON object in task output"));
}

#[test]
fn json_has_required_report_fields_rejects_non_object_json() {
    assert!(!json_has_required_report_fields(
        r#"["outcome","summary","details"]"#
    ));
}

#[test]
fn validate_task_artifact_flags_blockers_summary_and_changed_file_mismatches() {
    let fixture = CoordinationTestFixture::named("artifact-validation");
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
        CoordinationTaskStatus::Running,
    );
    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: String::new(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "   ".to_string(),
            details: String::new(),
            changed_files: vec!["src/lib.rs".to_string()],
            blockers: vec!["blocked".to_string()],
            ..Default::default()
        },
    };

    let errors =
        validate_task_artifact(&run.run_id, &task, &artifact).expect("validation should run");
    assert!(errors
        .iter()
        .any(|error| error.contains("missing task summary")));
    assert!(errors
        .iter()
        .any(|error| error.contains("blockers were reported")));
    assert!(errors
        .iter()
        .any(|error| error.contains("completed_no_change reported changed files")));
}

#[test]
fn validate_task_artifact_detects_stub_markers_and_missing_review_fix_tasks() {
    let fixture = CoordinationTestFixture::named("artifact-review-validation");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "review-task",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "fn sample() { todo!() }\n",
    )
    .unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "review".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "review".to_string(),
            details: "needs follow-up".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            blockers: vec!["needs patch".to_string()],
            remaining_gaps: vec!["follow-up".to_string()],
            ..Default::default()
        },
    };

    let errors =
        validate_task_artifact(&run.run_id, &task, &artifact).expect("validation should run");
    assert!(errors
        .iter()
        .any(|error| error.contains("still contains blocked stub markers")));
    assert!(errors
        .iter()
        .any(|error| error.contains("did not emit fix_tasks")));
}

#[test]
fn validate_task_artifact_rejects_missing_materialized_design_doc() {
    let fixture = CoordinationTestFixture::named("artifact-missing-design-doc");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let task = fixture.make_task(
        &run,
        "materialize",
        CoordinationTaskKind::MaterializeDesignDoc,
        Some("codex"),
        "Write canonical design doc to the workspace",
        &["artifacts/**"],
        &["docs/autonomous/design-doc.md"],
        &[],
        CoordinationTaskStatus::Running,
    );

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::Design),
        fallback_from: None,
        summary: "Wrote canonical design doc".to_string(),
        body: "doc contents".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Wrote canonical design doc".to_string(),
            details: "Replaced docs/autonomous/design-doc.md".to_string(),
            changed_files: vec!["docs/autonomous/design-doc.md".to_string()],
            ..Default::default()
        },
    };

    let errors =
        validate_task_artifact(&run.run_id, &task, &artifact).expect("validation should run");
    assert!(errors.iter().any(|error| {
        error.contains("materialize design doc reported changed file")
            && error.contains("docs/autonomous/design-doc.md")
    }));
}

#[test]
fn validate_task_artifact_rejects_summary_only_markdown_write_artifact_body() {
    let fixture = CoordinationTestFixture::named("artifact-summary-only-markdown");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "write-markdown",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write markdown artifact",
        &["docs/autonomous/design-doc.md"],
        &["docs/autonomous/prompts/track-2-cody-coleman.md"],
        &[],
        CoordinationTaskStatus::Running,
    );
    let output_path = fixture
        .workspace
        .path()
        .join("docs/autonomous/prompts/track-2-cody-coleman.md");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::write(
        &output_path,
        "# Actual document\n\n## Stage 0 Validation\n\nPass.\n",
    )
    .unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Wrote Track 2".to_string(),
        body: "Created docs/autonomous/prompts/track-2-cody-coleman.md with all required sections.".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Wrote Track 2".to_string(),
            details: "Created docs/autonomous/prompts/track-2-cody-coleman.md with all required sections.".to_string(),
            changed_files: vec!["docs/autonomous/prompts/track-2-cody-coleman.md".to_string()],
            ..Default::default()
        },
    };

    let errors =
        validate_task_artifact(&run.run_id, &task, &artifact).expect("validation should run");
    assert!(errors.iter().any(|error| {
        error.contains("did not contain the full document text")
            && error.contains("docs/autonomous/prompts/track-2-cody-coleman.md")
    }));
}

#[test]
fn prompt_narrative_artifact_accepts_heading_free_full_body_when_it_matches_file() {
    let fixture = CoordinationTestFixture::named("prompt-body-match");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "prompt-narrative",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write prompt narrative",
        &["docs/autonomous/design-doc.md"],
        &["docs/autonomous/prompts/prompt-01-platform-infra.md"],
        &[],
        CoordinationTaskStatus::Running,
    );
    let output_path = fixture
        .workspace
        .path()
        .join("docs/autonomous/prompts/prompt-01-platform-infra.md");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    let document = "Start from the platform engineer who scaled a visual-discovery service before 2014, pivot through archived engineering blog migrations, then cross-reference the surviving system names against the official GitHub organization and compare release dates against January 1, 2016. Report only the exact repository name.";
    fs::write(&output_path, format!("{document}\n")).unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Wrote prompt narrative".to_string(),
        body: document.to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Wrote prompt narrative".to_string(),
            details: document.to_string(),
            changed_files: vec!["docs/autonomous/prompts/prompt-01-platform-infra.md".to_string()],
            ..Default::default()
        },
    };

    let errors =
        validate_task_artifact(&run.run_id, &task, &artifact).expect("validation should run");
    assert!(!errors.iter().any(|error| {
        error.contains("did not contain the full document text")
            && error.contains("docs/autonomous/prompts/prompt-01-platform-infra.md")
    }));
}

#[test]
fn retry_scope_candidates_from_previous_scope_failure_deduplicates_additions() {
    let fixture = CoordinationTestFixture::named("retry-scope");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "retry",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Retry",
        &["src/lib.rs", "src/data.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "retry".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "retry".to_string(),
            details: String::new(),
            changed_files: vec![
                "src/data.rs".to_string(),
                "src/data.rs".to_string(),
                "src/lib.rs".to_string(),
            ],
            validation_errors: vec![
                "changed file 'src/data.rs' is outside declared write scope [\"src/lib.rs\"]"
                    .to_string(),
            ],
            ..Default::default()
        },
    });

    let additions = retry_scope_candidates_from_previous_scope_failure(&run.run_id, &task)
        .expect("retry scope");
    assert_eq!(additions, vec!["src/data.rs".to_string()]);
}

#[test]
fn scope_entry_for_changed_file_falls_back_for_outside_paths() {
    let fixture = CoordinationTestFixture::named("scope-entry");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    assert_eq!(
        scope_entry_for_changed_file(&task, "/tmp/outside.rs"),
        "tmp/outside.rs"
    );
}

#[test]
fn resolve_scope_to_cwd_and_workspace_relative_path_handle_absolute_workspace_paths() {
    let fixture = CoordinationTestFixture::named("scope-resolve");
    let cwd = fixture.workspace.path();
    let absolute = cwd.join("src/lib.rs");
    let absolute_like = absolute
        .to_string_lossy()
        .trim_start_matches('/')
        .to_string();

    assert_eq!(
        resolve_scope_to_cwd(cwd, &absolute_like),
        "src/lib.rs".to_string()
    );
    assert_eq!(
        workspace_relative_path(cwd, absolute.to_string_lossy().as_ref()),
        Some("src/lib.rs".to_string())
    );
}

#[test]
fn file_contains_stub_markers_detects_todo_files() {
    let fixture = CoordinationTestFixture::named("stub-markers");
    fs::write(
        fixture.workspace.path().join("src/stub.rs"),
        "pub fn sample() { unimplemented!() }\n",
    )
    .unwrap();

    assert!(
        file_contains_stub_markers(fixture.workspace.path(), "src/stub.rs")
            .expect("marker detection should succeed")
    );
}

#[test]
fn make_task_session_copies_task_identity() {
    let fixture = CoordinationTestFixture::named("task-session");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "session-task",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Session task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let session = make_task_session(&task);
    assert_eq!(session.id, task.task_id);
    assert_eq!(session.name, "auto-task-Session task");
    assert_eq!(session.launched_from.as_deref(), Some(task.cwd.as_str()));
    assert!(session.messages.is_empty());
}

#[test]
fn execute_builtin_task_rejects_provider_backed_tasks() {
    let fixture = CoordinationTestFixture::named("builtin-reject");
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

    let err = execute_builtin_task(&task).expect_err("provider-backed task should error");
    assert!(err.to_string().contains("provider-backed task"));
}

#[test]
fn collect_stream_response_prefers_final_non_empty_and_last_response_fallback() {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::stream::StreamMessage {
        response: "partial".to_string(),
        command: "codex exec".to_string(),
        session_id: None,
        summary: None,
        is_final: false,
    })
    .unwrap();
    tx.send(crate::stream::StreamMessage {
        response: "final".to_string(),
        command: "codex exec".to_string(),
        session_id: None,
        summary: None,
        is_final: true,
    })
    .unwrap();
    drop(tx);
    assert_eq!(collect_stream_response(rx), "final");

    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(crate::stream::StreamMessage {
        response: String::new(),
        command: "codex exec".to_string(),
        session_id: None,
        summary: None,
        is_final: false,
    })
    .unwrap();
    drop(tx);
    assert_eq!(collect_stream_response(rx), "");
}

#[test]
fn extract_provider_cooldown_supports_reset_phrase_and_unknown_providers() {
    let now = Utc::now();
    let response = "Gemini command failed. Your quota will reset after 2h3m4s.";
    let cooldown =
        extract_provider_cooldown("gemini", response, now).expect("reset phrase should parse");
    assert_eq!(cooldown.reason, "Gemini API quota exhausted.");
    assert_eq!(
        cooldown.unavailable_until - now,
        chrono::Duration::seconds(2 * 3600 + 3 * 60 + 4)
    );

    assert!(extract_provider_cooldown("claude", response, now).is_none());
    assert!(extract_provider_cooldown("unknown", response, now).is_none());
}

#[test]
fn extract_retry_delay_ms_and_parse_compact_duration_reject_invalid_values() {
    assert_eq!(extract_retry_delay_ms("retryDelayMs: 0"), None);
    assert_eq!(extract_retry_delay_ms("no retry delay here"), None);
    assert_eq!(
        extract_retry_delay_duration("{\"retryDelay\": \"40s\"}"),
        Some(chrono::Duration::seconds(40))
    );
    assert_eq!(
        extract_retry_delay_duration("Please retry in 40.025771073s."),
        Some(chrono::Duration::seconds(41))
    );
    assert_eq!(
        parse_compact_duration("1h2m3s"),
        Some(chrono::Duration::seconds(3723))
    );
    assert_eq!(
        parse_retry_delay_duration("40.025771073s"),
        Some(chrono::Duration::seconds(41))
    );
    assert_eq!(parse_compact_duration("15"), None);
    assert_eq!(extract_quota_reset_delay("resets soon"), None);
    assert_eq!(extract_retry_delay_duration("no retry delay here"), None);
}

#[test]
fn requeue_task_for_provider_cooldown_returns_false_for_inactive_or_unowned_claims() {
    let fixture = CoordinationTestFixture::named("requeue-false");
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.status = CoordinationRunStatus::Completed;
    let mut task = fixture.make_task(
        &run,
        "provider-task",
        CoordinationTaskKind::GeneratedReadTask,
        Some("gemini"),
        "Provider task",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(7);
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "gemini".to_string(),
        instance_pid: 7,
        launcher_pid: Some(7),
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

    let message = "Gemini command failed. TerminalQuotaError: exhausted quota. retryDelayMs: 5000";
    assert!(
        !requeue_task_for_provider_cooldown(&run.run_id, &task.task_id, 7, "gemini", message)
            .expect("inactive run should return false")
    );

    run.status = CoordinationRunStatus::Active;
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    let claim_file = claim_path(&run.run_id, &task.task_id);
    fs::remove_file(&claim_file).unwrap();
    assert!(
        !requeue_task_for_provider_cooldown(&run.run_id, &task.task_id, 7, "gemini", message)
            .expect("missing claim should return false")
    );

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "gemini".to_string(),
        instance_pid: 99,
        launcher_pid: Some(99),
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
    assert!(
        !requeue_task_for_provider_cooldown(&run.run_id, &task.task_id, 7, "gemini", message)
            .expect("wrong owner should return false")
    );
}

#[test]
fn task_report_validator_hint_and_provider_query_cover_remaining_kinds() {
    let fixture = CoordinationTestFixture::named("validator-hint");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);

    let survey_task = fixture.make_task(
        &run,
        "survey",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    assert!(task_report_validator_hint(&survey_task).is_none());
    let survey_query = build_provider_query(&survey_task).expect("survey query should build");
    assert!(survey_query.contains(&survey_task.prompt));
    assert!(survey_query.contains("Return JSON only"));

    let verify_task = fixture.make_task(
        &run,
        "verify",
        CoordinationTaskKind::RalphVerify,
        Some("codex"),
        "Verify",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    let hint = task_report_validator_hint(&verify_task).expect("ralph verify hint should exist");
    assert!(hint.contains("--kind ralph_verify"));
    assert!(!hint.contains("--workspace-root"));

    let verify_contract = provider_response_contract(&verify_task);
    assert!(verify_contract.contains("The JSON object must contain `complete`"));

    let mut plan_task = fixture.make_task(
        &run,
        "plan",
        CoordinationTaskKind::RalphPlan,
        Some("codex"),
        "Plan",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    plan_task.acceptance_checks = vec!["criterion one".to_string(), "criterion two".to_string()];
    let plan_contract = provider_response_contract(&plan_task);
    assert!(plan_contract.contains("Raw Acceptance Criteria"));
    assert!(plan_contract.contains("Status Board"));
    assert!(plan_contract.contains("Current Focus"));
    assert!(plan_contract.contains("criterion one"));

    let write_task = fixture.make_task(
        &run,
        "write",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Queued,
    );
    let _env = EnvVarGuard::set("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV", "runsc");
    let runsc_hint = task_report_validator_hint(&write_task).expect("write hint should exist");
    assert!(runsc_hint.contains("--workspace-root ."));
}
