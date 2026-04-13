use super::*;
struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}
impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }}
impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}
fn write_running_claim_fixture(
    home: &tempfile::TempDir,
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    worker_pid: u32,
    lifecycle_phase: ClaimLifecyclePhase,
    last_heartbeat_at: DateTime<Utc>,
) {
    let now = last_heartbeat_at;
    fs::create_dir_all(tasks_dir(run_id)).unwrap();
    fs::create_dir_all(claims_dir(run_id)).unwrap();
    write_json_atomic(
        &run_path(run_id),
        &CoordinationRun {
            run_id: run_id.to_string(),
            root_chat_id: "chat".to_string(),
            workspace_path: home.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Runsc,
            runsc_workspace_includes: vec!["instructions/".to_string()],
            provider: "codex".to_string(),
            goal: "test".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: now,
            updated_at: now,
            created_by_instance: None,
            max_agents: 1,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
            provider_cooldowns: BTreeMap::new(),
            task_ids: vec![task_id.to_string()],
            processed_mutation_batch_ids: BTreeSet::new(),
        },
    )
    .unwrap();
    write_json_atomic(
        &task_path(run_id, task_id),
        &CoordinationTask {
            task_id: task_id.to_string(),
            run_id: run_id.to_string(),
            title: "Provider task".to_string(),
            prompt: "prompt".to_string(),
            cwd: home.path().display().to_string(),
            kind: CoordinationTaskKind::DraftDesignDoc,
            assigned_provider: Some("codex".to_string()),
            status: CoordinationTaskStatus::Running,
            attempt_count: 1,
            max_attempts: 3,
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::DraftDesignDoc,
            )),
            claimed_by: Some(worker_pid),
            created_at: now,
            updated_at: now,
            ..CoordinationTask::default()
        },
    )
    .unwrap();
    write_json_atomic(
        &claim_path(run_id, task_id),
        &TaskClaim {
            run_id: run_id.to_string(),
            task_id: task_id.to_string(),
            provider: "codex".to_string(),
            instance_pid: worker_pid,
            launcher_pid: Some(launcher_pid),
            lifecycle_phase,
            worker_pid: Some(worker_pid),
            claimed_at: now,
            worker_spawned_at: Some(now),
            worker_adopted_at: Some(now),
            execution_started_at: Some(now),
            worker_exited_at: None,
            worker_exit_detail: None,
            last_heartbeat_at,
        },
    )
    .unwrap();
}
