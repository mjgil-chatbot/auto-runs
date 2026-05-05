#[test]
fn lightweight_claim_allows_parallel_overlap_on_active_task_doc() {
    let fixture = CoordinationTestFixture::named("lightweight-parallel-task-doc");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut owner = fixture.make_task_with_options(
        &run,
        "owner",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Owner task",
        vec!["functions.apply_patch".to_string()],
        vec!["src/app.js".to_string()],
        vec![
            "src/app.js".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    owner.claimed_by = Some(51);
    
    let candidate = fixture.make_task_with_options(
        &run,
        "candidate",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Candidate task",
        vec!["functions.apply_patch".to_string()],
        vec!["src/parser.js".to_string()],
        vec![
            "src/parser.js".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[owner.clone(), candidate.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: owner.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 51,
        launcher_pid: Some(51),
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

    let claimed = claim_next_runnable_task(&run.run_id, 61, "minimax")
        .unwrap()
        .expect("lightweight candidate should claim despite shared task doc");
    assert_eq!(claimed.task_id, candidate.task_id);
}

#[test]
fn lightweight_claim_allows_task_doc_overlap_with_broad_docs_scope_only() {
    let fixture = CoordinationTestFixture::named("lightweight-docs-overlap");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut owner = fixture.make_task_with_options(
        &run,
        "owner",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Owner docs task",
        vec!["functions.apply_patch".to_string()],
        vec!["docs/autonomous/lightweight-implementation-doc.md".to_string()],
        vec!["docs/autonomous".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    owner.claimed_by = Some(71);
    
    let candidate = fixture.make_task_with_options(
        &run,
        "candidate",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Candidate parser task",
        vec!["functions.apply_patch".to_string()],
        vec!["src/parser.js".to_string()],
        vec![
            "src/parser.js".to_string(),
            "docs/autonomous/lightweight-tasks.md".to_string(),
        ],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[owner.clone(), candidate.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: owner.task_id.clone(),
        provider: "minimax".to_string(),
        instance_pid: 71,
        launcher_pid: Some(71),
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

    let claimed = claim_next_runnable_task(&run.run_id, 81, "minimax")
        .unwrap()
        .expect("task should claim when overlap is only the coordination doc");
    assert_eq!(claimed.task_id, candidate.task_id);
}

#[test]
fn lightweight_generated_write_claim_ignores_repo_wide_scope_conflicts() {
    let fixture = CoordinationTestFixture::named("lightweight-repo-wide-overlap");
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["minimax".to_string()],
    );
    let mut owner = fixture.make_task_with_options(
        &run,
        "owner",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Owner repo-wide task",
        vec!["functions.apply_patch".to_string()],
        vec!["**".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    owner.claimed_by = Some(91);

    let candidate = fixture.make_task_with_options(
        &run,
        "candidate",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("minimax"),
        "Candidate repo-wide task",
        vec!["functions.apply_patch".to_string()],
        vec!["**".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[owner.clone(), candidate.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: owner.task_id.clone(),
        provider: "minimax".to_string(),
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

    let claimed = claim_next_runnable_task(&run.run_id, 92, "minimax")
        .unwrap()
        .expect("repo-wide lightweight write task should still claim");
    assert_eq!(claimed.task_id, candidate.task_id);
}
