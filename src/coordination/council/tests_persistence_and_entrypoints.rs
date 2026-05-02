fn persist_council_state(
    council: &CouncilRun,
    status: coordination::CoordinationRunStatus,
) -> coordination::CoordinationRun {
    let run = coordination::CoordinationRun {
        run_id: council.run_id.clone(),
        root_chat_id: council.root_chat_id.clone(),
        workspace_path: council.workspace_path.clone(),
        execution_environment: crate::AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: council.goal.clone(),
        workflow_mode: crate::AutonomousWorkflowMode::FreeformCouncil,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: council.completion_criteria.clone(),
        max_iterations: 1,
        status,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: council.allow_write_tasks,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: council.notes.clone(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: Some(council.turn_policy.clone()),
        council_max_turns: Some(council.max_turns),
        provider_cooldowns: std::collections::BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: std::collections::BTreeSet::new(),
    };
    coordination::write_json_atomic(&coordination::run_path(&council.run_id), &run).unwrap();
    coordination::write_json_atomic(&coordination::council_json_path(&council.run_id), council)
        .unwrap();
    run
}

fn cleanup_council_run(run_id: &str) {
    let _ = fs::remove_dir_all(run_dir(run_id));
}

fn pending_commit_path(run_id: &str) -> std::path::PathBuf {
    coordination::council_json_path(run_id).with_file_name("council-commit.json")
}

#[test]
fn load_council_run_and_ledger_handle_partial_trailing_line() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    persist_council_state(&council, coordination::CoordinationRunStatus::Active);

    let entry = CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::Discussion,
        "completed one turn",
    );
    let ledger_path = coordination::ledger_path(&council.run_id);
    fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
    fs::write(
        &ledger_path,
        format!(
            "{}\n{{\"partial\":true",
            serde_json::to_string(&entry).unwrap()
        ),
    )
    .unwrap();

    let loaded = load_council_run(&council.run_id).expect("council should load");
    assert_eq!(loaded.run_id, council.run_id);

    let ledger = load_council_ledger(&council.run_id).expect("ledger should load");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].entry_id, entry.entry_id);

    cleanup_council_run(&council.run_id);
}

#[test]
fn load_council_ledger_rejects_invalid_non_terminal_line() {
    let run_id = format!("council-ledger-invalid-{}", Uuid::new_v4().simple());
    let ledger_path = coordination::ledger_path(&run_id);
    fs::create_dir_all(ledger_path.parent().unwrap()).unwrap();
    fs::write(&ledger_path, "{\"broken\":true}\n{}\n").unwrap();

    let err = load_council_ledger(&run_id).expect_err("invalid non-terminal line should fail");
    assert!(err.to_string().contains("failed to parse ledger line"));

    cleanup_council_run(&run_id);
}

#[test]
fn public_council_entrypoints_finalize_pending_commit_and_cleanup() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.status = CouncilRunStatus::WaitingForUser;
    let run = persist_council_state(&council, coordination::CoordinationRunStatus::Active);
    let entries = vec![
        CouncilEntry::new(
            CouncilSpeaker::Claude,
            CouncilEntryKind::Discussion,
            "first discussion",
        ),
        CouncilEntry::new(
            CouncilSpeaker::System,
            CouncilEntryKind::SystemNudge,
            "nudge",
        ),
    ];

    coordination::write_json_atomic(
        &pending_commit_path(&council.run_id),
        &serde_json::json!({
            "council": council,
            "run": run,
            "entries": entries,
        }),
    )
    .unwrap();

    add_council_user_interjection(&council.run_id, "resume after pending commit")
        .expect("public entrypoint should finalize pending commit first");

    assert!(!pending_commit_path(&council.run_id).exists());
    assert_eq!(
        load_council_ledger(&council.run_id)
            .expect("ledger should load")
            .len(),
        3
    );
    assert!(
        coordination::load_events(&council.run_id, None, None)
            .expect("events should load")
            .len()
            >= 3
    );

    cleanup_council_run(&council.run_id);
}

#[test]
fn add_council_user_interjection_revokes_votes_and_reactivates_run() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.status = CouncilRunStatus::WaitingForUser;
    council.votes_complete = vec![CouncilSpeaker::Claude, CouncilSpeaker::Codex];
    persist_council_state(&council, coordination::CoordinationRunStatus::Active);

    add_council_user_interjection(
        &council.run_id,
        "@gemini please revisit this [REVOKE_VOTE claude]",
    )
    .expect("interjection should append");

    let updated = load_council_run(&council.run_id).expect("updated council should load");
    assert_eq!(updated.status, CouncilRunStatus::Active);
    assert_eq!(updated.turns_since_progress, 0);
    assert_eq!(updated.votes_complete, vec![CouncilSpeaker::Codex]);

    let ledger = load_council_ledger(&council.run_id).expect("ledger should load");
    assert!(ledger
        .iter()
        .any(|entry| entry.kind == CouncilEntryKind::UserInterjection));
    assert!(ledger
        .iter()
        .any(|entry| entry.kind == CouncilEntryKind::SystemNudge));

    cleanup_council_run(&council.run_id);
}

#[test]
fn drive_council_turn_handles_canceled_and_terminal_stall_states() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    persist_council_state(&council, coordination::CoordinationRunStatus::Canceled);

    let progressed = drive_council_turn(&council.run_id, 41).expect("turn should run");
    assert!(!progressed);
    assert_eq!(
        load_council_run(&council.run_id)
            .expect("council should reload")
            .status,
        CouncilRunStatus::Canceled
    );
    cleanup_council_run(&council.run_id);

    let workspace = TempDir::new().unwrap();
    council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.turns_since_progress = 4;
    council.termination_policy = CouncilTerminationPolicy {
        any: vec![TerminationCondition::MaxStallTurns(4)],
        all: Vec::new(),
    };
    persist_council_state(&council, coordination::CoordinationRunStatus::Active);

    let progressed = drive_council_turn(&council.run_id, 41).expect("stall turn should run");
    assert!(progressed);
    let updated = load_council_run(&council.run_id).expect("council should reload");
    assert_eq!(updated.status, CouncilRunStatus::Failed);
    assert!(load_council_ledger(&council.run_id)
        .expect("ledger should load")
        .iter()
        .any(|entry| entry.content.contains("stalled")));

    cleanup_council_run(&council.run_id);
}

#[test]
fn seed_council_run_persists_termination_policy_from_run_max_turns() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let run = coordination::CoordinationRun {
        run_id: format!("council-seed-{}", Uuid::new_v4().simple()),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.path().display().to_string(),
        execution_environment: crate::AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "seed council".to_string(),
        workflow_mode: crate::AutonomousWorkflowMode::FreeformCouncil,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: "done".to_string(),
        max_iterations: 1,
        status: coordination::CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: Some(TurnPolicy::RoundRobin),
        council_max_turns: Some(17),
        provider_cooldowns: std::collections::BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: std::collections::BTreeSet::new(),
    };

    coordination::write_json_atomic(&coordination::run_path(&run.run_id), &run).unwrap();
    coordination::seed_council_run(&run).expect("seed should persist council state");

    let council = load_council_run(&run.run_id).expect("seeded council should load");
    assert_eq!(council.max_turns, 17);
    assert!(council
        .termination_policy
        .any
        .contains(&TerminationCondition::MaxTurns(17)));
    assert!(council
        .termination_policy
        .any
        .contains(&TerminationCondition::MaxStallTurns(TERMINAL_STALL_TURNS)));
    assert_eq!(
        council.termination_policy.all,
        vec![TerminationCondition::AllModelsVotedComplete]
    );

    cleanup_council_run(&run.run_id);
}

#[test]
fn add_council_user_interjection_recovers_pristine_missing_council_state() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let run = coordination::CoordinationRun {
        run_id: format!("council-recover-{}", Uuid::new_v4().simple()),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.path().display().to_string(),
        execution_environment: crate::AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "recover missing council state".to_string(),
        workflow_mode: crate::AutonomousWorkflowMode::FreeformCouncil,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: "done".to_string(),
        max_iterations: 1,
        status: coordination::CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: Some(TurnPolicy::RoundRobin),
        council_max_turns: Some(10),
        provider_cooldowns: std::collections::BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: std::collections::BTreeSet::new(),
    };
    coordination::write_json_atomic(&coordination::run_path(&run.run_id), &run).unwrap();

    add_council_user_interjection(&run.run_id, "resume council").unwrap();

    let council = load_council_run(&run.run_id).expect("council should be reseeded");
    assert_eq!(council.status, CouncilRunStatus::Active);
    let ledger = load_council_ledger(&run.run_id).expect("ledger should load");
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].kind, CouncilEntryKind::UserInterjection);
    let events = coordination::load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "council_state_recovered"));

    cleanup_council_run(&run.run_id);
}

#[test]
fn drive_council_turn_quarantines_missing_council_state_when_workspace_is_gone() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let missing_workspace = workspace.path().display().to_string();
    drop(workspace);

    let run = coordination::CoordinationRun {
        run_id: format!("council-quarantine-{}", Uuid::new_v4().simple()),
        root_chat_id: "chat".to_string(),
        workspace_path: missing_workspace,
        execution_environment: crate::AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "quarantine missing council state".to_string(),
        workflow_mode: crate::AutonomousWorkflowMode::FreeformCouncil,
        design_doc_path: "docs/design.md".to_string(),
        completion_criteria: "done".to_string(),
        max_iterations: 1,
        status: coordination::CoordinationRunStatus::Active,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        created_by_instance: None,
        max_agents: 4,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: false,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: Some(TurnPolicy::RoundRobin),
        council_max_turns: Some(10),
        provider_cooldowns: std::collections::BTreeMap::new(),
        task_ids: Vec::new(),
    processed_mutation_batch_ids: std::collections::BTreeSet::new(),
    };
    coordination::write_json_atomic(&coordination::run_path(&run.run_id), &run).unwrap();

    assert!(drive_council_turn(&run.run_id, 41).unwrap());

    let updated_run = coordination::load_run(&run.run_id).unwrap();
    assert_eq!(
        updated_run.status,
        coordination::CoordinationRunStatus::Failed
    );
    let council = load_council_run(&run.run_id).expect("quarantined council should load");
    assert_eq!(council.status, CouncilRunStatus::Failed);
    let events = coordination::load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "council_state_corrupt"));

    cleanup_council_run(&run.run_id);
}

#[test]
fn make_council_provider_session_uses_run_workspace_as_launch_dir() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run = persist_council_state(&council, coordination::CoordinationRunStatus::Active);

    let session = super::runtime::make_council_provider_session(CouncilSpeaker::Codex, &run);

    assert_eq!(
        session.launched_from.as_deref(),
        Some(run.workspace_path.as_str())
    );
    assert_eq!(session.id, format!("council-{}-codex", run.run_id));

    cleanup_council_run(&council.run_id);
}
