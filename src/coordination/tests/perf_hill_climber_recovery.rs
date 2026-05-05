#[test]
fn perf_bench_build_failure_is_reported_for_evaluate_instead_of_failing_run() {
    let report = perf_bench_report(
        r#"{"idea_index":3,"build_failed":true,"error":"rust-lld: error: unable to find library -luring-ffi"}"#,
    )
    .expect("build-failed benchmark artifact should parse");

    assert_eq!(report.outcome, CoordinationTaskOutcome::CompletedWithChanges);
    assert_eq!(report.summary, "Bench idea 4: release build failed");
    assert_eq!(
        report.changed_files,
        vec!["/tmp/perf_bench_3.json".to_string()]
    );
    assert!(report.details.contains("-luring-ffi"));
}

#[test]
fn perf_benchmark_prompts_treat_bench_script_json_as_authoritative() {
    let fixture = CoordinationTestFixture::named("perf-benchmark-prompts");
    let run = fixture.make_run(AutonomousWorkflowMode::PerfHillClimber);
    let baseline_prompt = build_perf_baseline_prompt(&run);
    let idea = PerfIdea {
        index: 0,
        title: "cleanup".to_string(),
        description: "revert rejected idea".to_string(),
    };
    let bench_prompt = build_perf_bench_prompt(&idea, 0, &run);

    assert!(baseline_prompt.contains("already emits the canonical perf benchmark JSON"));
    assert!(baseline_prompt.contains("Write that exact JSON stdout"));
    assert!(bench_prompt.contains("PERF_IDEA_INDEX=0"));
    assert!(bench_prompt.contains("Return that exact same JSON object"));
}

#[test]
fn perf_evaluate_prompt_reverts_tracked_and_untracked_files() {
    let fixture = CoordinationTestFixture::named("perf-evaluate-cleanup");
    let run = fixture.make_run(AutonomousWorkflowMode::PerfHillClimber);
    let idea = PerfIdea {
        index: 0,
        title: "cleanup".to_string(),
        description: "revert rejected idea".to_string(),
    };

    let prompt = build_perf_evaluate_prompt(&idea, 0, &run);

    assert!(prompt.contains("git checkout -- . && git clean -fd"));
    assert!(prompt.contains("correctness.status"));
    assert!(prompt.contains("metrics.primary_case"));
}

fn perf_parse_artifact(run: &CoordinationRun, task_id: &str) -> TaskArtifact {
    TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task_id.to_string(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Parsed 2 performance improvement ideas".to_string(),
        body: r#"{"ideas":[
            {"index":0,"title":"first idea","description":"first change"},
            {"index":1,"title":"second idea","description":"second change"}
        ]}"#
        .to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Parsed 2 performance improvement ideas".to_string(),
            details: "ideas".to_string(),
            ..Default::default()
        },
    }
}

fn run_git_in_test_workspace(workspace: &std::path::Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

fn init_perf_recovery_git_workspace(workspace: &std::path::Path) {
    run_git_in_test_workspace(workspace, &["init"]);
    run_git_in_test_workspace(workspace, &["config", "user.email", "tests@example.com"]);
    run_git_in_test_workspace(workspace, &["config", "user.name", "Coordination Tests"]);
    run_git_in_test_workspace(workspace, &["add", "."]);
    run_git_in_test_workspace(workspace, &["commit", "-m", "initial"]);
}

fn make_perf_recovery_fixture() -> (
    CoordinationTestFixture,
    CoordinationRun,
    CoordinationTask,
    CoordinationTask,
    CoordinationTask,
    CoordinationTask,
    CoordinationTask,
) {
    let fixture = CoordinationTestFixture::named("perf-failure-recovery");
    init_perf_recovery_git_workspace(fixture.workspace.path());
    let mut run = fixture.make_run(AutonomousWorkflowMode::PerfHillClimber);
    run.auto_retry_failed_tasks = false;
    run.notes = format!(
        "perf_worktree_root={}\nperf_worktree_path={}",
        fixture.workspace.path().display(),
        fixture.workspace.path().display()
    );
    let baseline = fixture.make_task(
        &run,
        "baseline",
        CoordinationTaskKind::PerfBaseline,
        Some("codex"),
        "Capture performance baseline",
        &["."],
        &["/tmp/perf_baseline.json"],
        &[],
        CoordinationTaskStatus::Completed,
    );
    let parse = fixture.make_task(
        &run,
        "parse",
        CoordinationTaskKind::PerfParse,
        Some("codex"),
        "Parse performance improvement ideas",
        &["**"],
        &[],
        &["baseline"],
        CoordinationTaskStatus::Completed,
    );
    let mut apply = fixture.make_task(
        &run,
        "apply-0",
        CoordinationTaskKind::PerfApply,
        Some("codex"),
        "Apply perf idea 1: first idea",
        &["**"],
        &["."],
        &["parse"],
        CoordinationTaskStatus::Running,
    );
    apply.iteration = Some(0);
    apply.claimed_by = Some(77);
    
    let mut bench = fixture.make_task(
        &run,
        "bench-0",
        CoordinationTaskKind::PerfBench,
        Some("codex"),
        "Bench perf idea 1: first idea",
        &["."],
        &["/tmp/perf_bench_0.json"],
        &["apply-0"],
        CoordinationTaskStatus::Blocked,
    );
    bench.iteration = Some(0);
    let mut evaluate = fixture.make_task(
        &run,
        "evaluate-0",
        CoordinationTaskKind::PerfEvaluate,
        Some("codex"),
        "Evaluate perf idea 1: first idea",
        &["/tmp/perf_baseline.json", "/tmp/perf_bench_0.json"],
        &["/tmp/perf_evaluate_0.json", "."],
        &["bench-0", "baseline"],
        CoordinationTaskStatus::Blocked,
    );
    evaluate.iteration = Some(0);

    fixture.persist_run_with_tasks(
        &run,
        &[baseline.clone(), parse.clone(), apply.clone(), bench.clone(), evaluate.clone()],
    );
    fixture.write_artifact(&perf_parse_artifact(&run, &parse.task_id));
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: apply.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
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

    (fixture, run, baseline, parse, apply, bench, evaluate)
}

#[test]
fn malformed_perf_apply_report_rejects_idea_and_advances() {
    let (_fixture, run, _baseline, _parse, apply, _bench, _evaluate) =
        make_perf_recovery_fixture();

    complete_task(
        &run.run_id,
        &apply.task_id,
        77,
        TaskArtifact {
            run_id: run.run_id.clone(),
            task_id: apply.task_id.clone(),
            provider: Some("codex".to_string()),
            task_category: None,
            fallback_from: None,
            summary: "Provider returned a malformed task report.".to_string(),
            body: "Rust normalized a malformed provider task report before recording the failure."
                .to_string(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::Failed,
                summary: "Provider returned a malformed task report.".to_string(),
                details: "missing JSON object in task output".to_string(),
                ..Default::default()
            },
        },
    )
    .unwrap();

    let recovered_apply = load_task(&run.run_id, &apply.task_id).unwrap();
    assert_eq!(recovered_apply.status, CoordinationTaskStatus::Completed);
    let recovered_bench = load_task(&run.run_id, "bench-0").unwrap();
    assert_eq!(recovered_bench.status, CoordinationTaskStatus::Completed);
    let recovered_eval = load_task(&run.run_id, "evaluate-0").unwrap();
    assert_eq!(recovered_eval.status, CoordinationTaskStatus::Completed);
    let next_apply = load_tasks(&run.run_id)
        .unwrap()
        .into_iter()
        .find(|task| task.kind == CoordinationTaskKind::PerfApply && task.iteration == Some(1))
        .expect("next perf idea should be spawned");
    assert_eq!(next_apply.status, CoordinationTaskStatus::Queued);
    let eval_artifact_raw =
        fs::read_to_string(artifact_result_path(&run.run_id, "evaluate-0")).unwrap();
    let eval_artifact: TaskArtifact = serde_json::from_str(&eval_artifact_raw).unwrap();
    assert_eq!(
        eval_artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert!(eval_artifact.body.contains(r#""keep":false"#));
}

#[test]
fn fail_task_for_perf_apply_rejects_idea_instead_of_failing_run() {
    let (_fixture, run, _baseline, _parse, apply, _bench, _evaluate) =
        make_perf_recovery_fixture();

    fail_task(
        &run.run_id,
        &apply.task_id,
        77,
        "Failed to spawn codex command: No such file or directory (os error 2)",
    )
    .unwrap();

    let recovered_apply = load_task(&run.run_id, &apply.task_id).unwrap();
    assert_eq!(recovered_apply.status, CoordinationTaskStatus::Completed);
    let snapshot = load_run_snapshot(&run.run_id).unwrap();
    assert_ne!(snapshot.run.status, CoordinationRunStatus::Failed);
    assert!(snapshot.tasks.iter().any(|task| {
        task.kind == CoordinationTaskKind::PerfApply
            && task.iteration == Some(1)
            && task.status == CoordinationTaskStatus::Queued
    }));
}

#[test]
fn perf_apply_failure_recovery_uses_human_facing_idea_numbers() {
    let (_fixture, run, _baseline, _parse, apply, _bench, _evaluate) =
        make_perf_recovery_fixture();

    fail_task(
        &run.run_id,
        &apply.task_id,
        77,
        "Codex command failed.",
    )
    .unwrap();

    let bench_artifact_raw =
        fs::read_to_string(artifact_result_path(&run.run_id, "bench-0")).unwrap();
    let bench_artifact: TaskArtifact = serde_json::from_str(&bench_artifact_raw).unwrap();
    assert_eq!(
        bench_artifact.report.summary,
        "Bench idea 1: skipped after apply failure"
    );

    let eval_artifact_raw =
        fs::read_to_string(artifact_result_path(&run.run_id, "evaluate-0")).unwrap();
    let eval_artifact: TaskArtifact = serde_json::from_str(&eval_artifact_raw).unwrap();
    assert!(eval_artifact.report.summary.starts_with("Idea 1 reverted: 0.0%"));
}
