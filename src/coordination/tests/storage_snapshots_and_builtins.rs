#[test]
fn storage_round_trips_global_provider_cooldowns() {
    let fixture = CoordinationTestFixture::new();
    let _run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let now = Utc::now();
    let mut cooldowns = BTreeMap::new();
    cooldowns.insert(
        "gemini".to_string(),
        ProviderCooldown {
            unavailable_until: now + chrono::Duration::minutes(5),
            reason: "quota exhausted".to_string(),
        },
    );

    write_global_provider_cooldowns(&cooldowns).unwrap();
    let loaded = load_global_provider_cooldowns().unwrap();

    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded["gemini"].reason, "quota exhausted");
}

#[test]
fn survey_workspace_builtin_renders_workspace_and_skips_ignored_directories() {
    let fixture = CoordinationTestFixture::new();
    fs::create_dir_all(fixture.workspace.path().join("src/nested")).unwrap();
    fs::create_dir_all(fixture.workspace.path().join("target/debug")).unwrap();
    fs::write(
        fixture.workspace.path().join("src/nested/lib.rs"),
        "pub fn ok() {}\n",
    )
    .unwrap();
    fs::write(
        fixture.workspace.path().join("target/debug/ignored.rs"),
        "pub fn ignored() {}\n",
    )
    .unwrap();

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
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let artifact = execute_survey_workspace(&task).unwrap();
    assert!(artifact.summary.contains("Surveyed workspace"));
    assert!(artifact.body.contains("src/nested/lib.rs"));
    assert!(!artifact.body.contains("target/debug/ignored.rs"));
    assert_eq!(
        artifact.report.design_requirements_satisfied,
        vec![
            "Collected current workspace context.".to_string(),
            "Captured a bounded sample of files and directories.".to_string(),
        ]
    );
}

#[test]
fn survey_workspace_builtin_creates_missing_workspace_and_logs_event() {
    let fixture = CoordinationTestFixture::new();
    let missing_workspace = fixture.workspace.path().join("missing-workspace");

    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.workspace_path = missing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "survey-missing",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let artifact = execute_survey_workspace(&task).unwrap();

    assert!(missing_workspace.is_dir());
    assert!(artifact.summary.contains("Surveyed workspace"));
    let events = load_events(&run.run_id, None, None).unwrap();
    let created = events
        .iter()
        .find(|event| event.event_type == "task_workspace_created")
        .expect("workspace creation event should be recorded");
    assert_eq!(created.task_id.as_deref(), Some(task.task_id.as_str()));
    assert_eq!(created.provider.as_deref(), Some("codex"));
    assert!(created
        .message
        .contains("Created missing workspace directory"));
    assert!(created.message.contains(&run.workspace_path));
}

#[cfg(all(target_os = "linux", unix))]
fn write_test_executable(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[cfg(target_os = "linux")]
fn install_runsc_survey_stubs(bin_dir: &Path) {
    write_test_executable(
        &bin_dir.join("docker"),
        r#"#!/usr/bin/env bash
set -euo pipefail
entrypoint=""
workdir=""
while (($#)); do
  case "$1" in
    run|--runtime=runsc|--rm|--init)
      shift
      ;;
    --entrypoint)
      entrypoint="$2"
      shift 2
      ;;
    --user|--workdir|--volume|--env)
      if [[ "$1" == "--workdir" ]]; then
        workdir="$2"
      fi
      shift 2
      ;;
    *)
      image="$1"
      shift
      break
      ;;
  esac
done
if [[ "$entrypoint" == "/usr/bin/ldd" ]]; then
  echo "ldd (GNU libc) 2.39"
  exit 0
fi
cd "${workdir:-.}"
exec "$@"
"#,
    );
    write_test_executable(&bin_dir.join("runsc"), "#!/usr/bin/env bash\nexit 0\n");
}

#[test]
#[cfg(target_os = "linux")]
fn survey_workspace_builtin_uses_runsc_staging_for_workspace_entries() {
    let fixture = CoordinationTestFixture::new();
    fs::create_dir_all(fixture.workspace.path().join("instructions")).unwrap();
    fs::create_dir_all(fixture.workspace.path().join("docs")).unwrap();
    fs::write(
        fixture.workspace.path().join("instructions/spec.md"),
        "# included\n",
    )
    .unwrap();
    fs::write(fixture.workspace.path().join("plan.txt"), "included plan\n").unwrap();
    fs::write(
        fixture.workspace.path().join("docs/leaked.md"),
        "# should stay hidden\n",
    )
    .unwrap();

    let bin_dir = tempfile::tempdir().unwrap();
    install_runsc_survey_stubs(bin_dir.path());
    let path_value = format!(
        "{}:{}",
        bin_dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path_guard = EnvVarGuard::set("PATH", path_value);

    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.execution_environment = AutonomousExecutionEnvironment::Runsc;
    run.runsc_workspace_includes = vec!["instructions".to_string(), "plan.txt".to_string()];
    let task = fixture.make_task(
        &run,
        "survey-runsc",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let artifact = execute_survey_workspace(&task).unwrap();
    assert!(artifact.summary.contains("via runsc"));
    assert!(artifact.body.contains("instructions/spec.md"));
    assert!(artifact.body.contains("plan.txt"));
    assert!(!artifact.body.contains("docs/leaked.md"));
}

#[test]
#[cfg(target_os = "linux")]
fn survey_workspace_builtin_respects_explicit_runsc_includes_for_design_consensus() {
    let fixture = CoordinationTestFixture::new();
    fs::create_dir_all(fixture.workspace.path().join("instructions")).unwrap();
    fs::create_dir_all(fixture.workspace.path().join("docs/autonomous")).unwrap();
    fs::create_dir_all(fixture.workspace.path().join("workflow-registry")).unwrap();
    fs::write(
        fixture.workspace.path().join("instructions/spec.md"),
        "# included\n",
    )
    .unwrap();
    fs::write(fixture.workspace.path().join("plan.txt"), "included plan\n").unwrap();
    fs::write(
        fixture.workspace.path().join("docs/autonomous/existing.md"),
        "# existing design\n",
    )
    .unwrap();
    fs::write(
        fixture
            .workspace
            .path()
            .join("workflow-registry/providers.json"),
        "{ \"providers\": [] }\n",
    )
    .unwrap();

    let bin_dir = tempfile::tempdir().unwrap();
    install_runsc_survey_stubs(bin_dir.path());
    let path_value = format!(
        "{}:{}",
        bin_dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path_guard = EnvVarGuard::set("PATH", path_value);

    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    run.execution_environment = AutonomousExecutionEnvironment::Runsc;
    run.runsc_workspace_includes = vec!["instructions".to_string(), "plan.txt".to_string()];
    let task = fixture.make_task(
        &run,
        "survey-runsc-design",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let artifact = execute_survey_workspace(&task).unwrap();
    assert!(artifact.body.contains("instructions/spec.md"));
    assert!(artifact.body.contains("plan.txt"));
    assert!(!artifact.body.contains("docs/autonomous/existing.md"));
    assert!(!artifact.body.contains("workflow-registry/providers.json"));
}

#[test]
#[cfg(target_os = "linux")]
fn survey_workspace_builtin_reads_shared_runsc_workspace_state() {
    let fixture = CoordinationTestFixture::new();
    fs::create_dir_all(fixture.workspace.path().join("instructions")).unwrap();
    fs::write(
        fixture.workspace.path().join("instructions/spec.md"),
        "# included\n",
    )
    .unwrap();

    let bin_dir = tempfile::tempdir().unwrap();
    install_runsc_survey_stubs(bin_dir.path());
    let path_value = format!(
        "{}:{}",
        bin_dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let _path_guard = EnvVarGuard::set("PATH", path_value);

    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.execution_environment = AutonomousExecutionEnvironment::Runsc;
    run.runsc_workspace_includes = vec!["instructions".to_string()];
    let task = fixture.make_task(
        &run,
        "survey-runsc-shared",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    ensure_autonomous_runsc_workspace(&run).unwrap();
    fs::write(
        autonomous_runsc_workspace_root(&run.run_id).join("instructions/from-shared.md"),
        "# shared\n",
    )
    .unwrap();

    let artifact = execute_survey_workspace(&task).unwrap();
    assert!(artifact.body.contains("instructions/spec.md"));
    assert!(artifact.body.contains("instructions/from-shared.md"));
}

#[test]
fn runsc_materialized_files_capture_from_shared_workspace_and_write_back_host() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    run.execution_environment = AutonomousExecutionEnvironment::Runsc;
    run.runsc_workspace_includes = vec!["src".to_string()];
    let task = fixture.make_task(
        &run,
        "write-runsc-shared",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write through shared runsc workspace",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );

    ensure_autonomous_runsc_workspace(&run).unwrap();
    let _runsc_guard = AutonomousRunscEnvGuard::activate(&run)
        .unwrap()
        .expect("runsc env guard should activate");
    fs::write(
        fixture.workspace.path().join("src/lib.rs"),
        "host version\n",
    )
    .unwrap();
    fs::write(
        autonomous_runsc_workspace_root(&run.run_id).join("src/lib.rs"),
        "sandbox version\n",
    )
    .unwrap();

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: task.task_category,
        fallback_from: None,
        summary: "Updated src/lib.rs".to_string(),
        body: "sandbox version\n".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Updated src/lib.rs".to_string(),
            details: "sandbox version\n".to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            ..Default::default()
        },
    };

    let files = capture_artifact_materialized_files(&task, &artifact);
    assert_eq!(
        files,
        vec![MaterializedArtifactFile {
            path: "src/lib.rs".to_string(),
            content: "sandbox version\n".to_string(),
        }]
    );

    write_materialized_files_to_workspace(&task, &files).unwrap();
    assert_eq!(
        fs::read_to_string(fixture.workspace.path().join("src/lib.rs")).unwrap(),
        "sandbox version\n"
    );
}

#[test]
fn code_review_materialization_builtin_writes_rendered_document() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::CodeReview);
    let review_task = fixture.make_task(
        &run,
        "review",
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review chunk",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let materialize_task = fixture.make_task(
        &run,
        "materialize",
        CoordinationTaskKind::MaterializeDesignDoc,
        None,
        "Materialize review",
        &["artifacts/**"],
        &["docs/code-review.md"],
        &["review"],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[review_task.clone(), materialize_task.clone()]);

    write_review_task_metadata(
        &run.run_id,
        &materialize_task.task_id,
        &code_review::ReviewTaskMetadata {
            scope_description: "Current workspace".to_string(),
            dimensions: vec!["architecture".to_string()],
            chunk: Vec::new(),
            automated_checks: vec![code_review::AutomatedToolCheck {
                command: "cargo test".to_string(),
                status: "passed".to_string(),
                summary: "ok".to_string(),
            }],
            profile_path: code_review::REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: vec!["profile warning".to_string()],
            fix_mode: Some(ReviewFixMode::ReportOnly),
            output_path: Some("docs/code-review.md".to_string()),
        },
    )
    .unwrap();

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "reviewed".to_string(),
        body: serde_json::to_string(&code_review::ReviewArtifactResponse {
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedNoChange,
                summary: "reviewed".to_string(),
                details: "done".to_string(),
                remaining_gaps: vec!["Need more tests".to_string()],
                ..Default::default()
            },
            findings: vec![code_review::ReviewFinding {
                severity: code_review::ReviewSeverity::High,
                confidence: "high".to_string(),
                file: "src/lib.rs".to_string(),
                line: 7,
                dimension: "architecture".to_string(),
                description: "State update is missing a guard".to_string(),
                fix_suggestion: "Add the missing check".to_string(),
            }],
        })
        .unwrap(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "reviewed".to_string(),
            details: "done".to_string(),
            ..Default::default()
        },
    });

    let artifact = execute_code_review_materialization(&materialize_task).unwrap();
    let written = fs::read_to_string(fixture.workspace.path().join("docs/code-review.md")).unwrap();

    assert!(written.contains("# Code Review"));
    assert!(written.contains("State update is missing a guard"));
    assert!(written.contains("cargo test: passed (ok)"));
    assert_eq!(
        artifact.report.changed_files,
        vec!["docs/code-review.md".to_string()]
    );
}

#[test]
fn load_run_snapshot_unblocks_blocked_tasks_and_marks_completed_runs() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let survey = fixture.make_task(
        &run,
        "survey",
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let plan = fixture.make_task(
        &run,
        "plan",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan",
        &["**"],
        &[],
        &["survey"],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[survey.clone(), plan.clone()]);

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let queued_plan = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == plan.task_id)
        .unwrap();
    assert_eq!(queued_plan.status, CoordinationTaskStatus::Queued);
    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);

    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: survey.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "surveyed".to_string(),
        body: "body".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "surveyed".to_string(),
            details: "body".to_string(),
            ..Default::default()
        },
    });

    let mut plan_completed = load_task(&run.run_id, &plan.task_id).unwrap();
    plan_completed.status = CoordinationTaskStatus::Completed;
    write_json_atomic(
        &task_path(&run.run_id, &plan_completed.task_id),
        &plan_completed,
    )
    .unwrap();
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: plan_completed.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "planned".to_string(),
        body: "body".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "planned".to_string(),
            details: "body".to_string(),
            ..Default::default()
        },
    });

    let completed_snapshot = load_run_snapshot(&run.run_id).unwrap();
    assert_eq!(
        completed_snapshot.run.status,
        CoordinationRunStatus::Completed
    );
}

#[test]
fn load_run_snapshot_reblocks_queued_tasks_with_unmet_dependencies() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let draft = fixture.make_task(
        &run,
        "draft",
        CoordinationTaskKind::DraftDesignDoc,
        Some("gemini"),
        "Draft provider design proposal with gemini-3-flash-preview",
        &["instructions/**", "plan.txt"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    let materialize = fixture.make_task(
        &run,
        "follow-up",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("gemini"),
        "Follow-up write task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &["draft"],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[draft.clone(), materialize.clone()]);
    persist_live_test_claim(&draft);

    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    let reblocked = snapshot
        .tasks
        .iter()
        .find(|task| task.task_id == materialize.task_id)
        .expect("materialize task should remain present");

    assert_eq!(reblocked.status, CoordinationTaskStatus::Blocked);
    assert_eq!(snapshot.run.status, CoordinationRunStatus::Active);
}

#[test]
fn complete_task_persists_provider_draft_design_doc_from_artifact_body() {
    let fixture = CoordinationTestFixture::new();
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let mut draft = fixture.make_task(
        &run,
        "draft",
        CoordinationTaskKind::DraftDesignDoc,
        Some("qwen"),
        "Draft provider design proposal with Qwen 3.6 Free",
        &["instructions/**", "plan.txt"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    draft.claimed_by = Some(91);

    fixture.persist_run_with_tasks(&run, &[draft.clone()]);

    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: draft.task_id.clone(),
        provider: "qwen".to_string(),
        instance_pid: 91,
        launcher_pid: Some(91),
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

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: draft.task_id.clone(),
        provider: Some("qwen".to_string()),
        task_category: Some(TaskCategory::Design),
        fallback_from: None,
        summary: "drafted".to_string(),
        body: "# Draft Proposal\n\nUse the artifact body verbatim.\n".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "drafted".to_string(),
            details: "# Draft Proposal\n\nUse the artifact body verbatim.\n".to_string(),
            ..Default::default()
        },
    };

    complete_task(&run.run_id, &draft.task_id, 91, artifact).unwrap();

    let doc_path = fixture
        .workspace
        .path()
        .join(provider_draft_design_doc_path(&run, "qwen"));
    assert_eq!(
        fs::read_to_string(doc_path).unwrap(),
        "# Draft Proposal\n\nUse the artifact body verbatim.\n"
    );
}

#[test]
fn drive_run_for_instance_executes_builtin_task_to_completion() {
    let fixture = CoordinationTestFixture::new();
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
    fixture.persist_run_with_tasks(&run, &[task.clone()]);

    let did_work = drive_run_for_instance(&run.run_id, 71, "codex").unwrap();

    assert!(did_work);
    let completed = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(completed.status, CoordinationTaskStatus::Completed);
    assert_eq!(completed.claimed_by, None);
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
    assert!(artifact_result_path(&run.run_id, &task.task_id).is_file());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_claimed"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_completed"));
}
