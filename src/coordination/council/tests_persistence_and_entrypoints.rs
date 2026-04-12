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
