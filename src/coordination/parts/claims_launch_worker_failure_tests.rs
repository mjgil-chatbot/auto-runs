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
    }
}

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

#[test]
fn test_release_claim_after_worker_failure_clears_adopted_claim() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let now = Utc::now();
    let run_id = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    let launcher_pid = 41;
    let worker_pid = 52;

    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    write_json_atomic(
        &run_path(&run_id),
        &CoordinationRun {
            run_id: run_id.clone(),
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
            task_ids: vec![task_id.clone()],
            processed_mutation_batch_ids: BTreeSet::new(),
        },
    )
    .unwrap();
    write_json_atomic(
        &task_path(&run_id, &task_id),
        &CoordinationTask {
            task_id: task_id.clone(),
            run_id: run_id.clone(),
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
        &claim_path(&run_id, &task_id),
        &TaskClaim {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
            provider: "codex".to_string(),
            instance_pid: worker_pid,
            launcher_pid: Some(launcher_pid),
            lifecycle_phase: ClaimLifecyclePhase::WorkerAdopted,
            worker_pid: Some(worker_pid),
            claimed_at: now,
            worker_spawned_at: Some(now),
            worker_adopted_at: Some(now),
            execution_started_at: None,
            worker_exited_at: None,
            worker_exit_detail: None,
            last_heartbeat_at: now,
        },
    )
    .unwrap();

    release_claim_after_worker_failure(
        &run_id,
        &task_id,
        worker_pid,
        &anyhow::anyhow!("worker died after adoption"),
    )
    .unwrap();

    let task = load_task(&run_id, &task_id).unwrap();
    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert_eq!(task.claimed_by, None);

    assert!(!claim_path(&run_id, &task_id).exists());
}

#[test]
fn test_release_claim_after_spawned_worker_exit_clears_unadopted_claim() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let now = Utc::now();
    let run_id = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    let launcher_pid = 41;
    let worker_pid = 52;

    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    write_json_atomic(
        &run_path(&run_id),
        &CoordinationRun {
            run_id: run_id.clone(),
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
            task_ids: vec![task_id.clone()],
            processed_mutation_batch_ids: BTreeSet::new(),
        },
    )
    .unwrap();
    write_json_atomic(
        &task_path(&run_id, &task_id),
        &CoordinationTask {
            task_id: task_id.clone(),
            run_id: run_id.clone(),
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
            claimed_by: Some(launcher_pid),
            created_at: now,
            updated_at: now,
            ..CoordinationTask::default()
        },
    )
    .unwrap();
    write_json_atomic(
        &claim_path(&run_id, &task_id),
        &TaskClaim {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
            provider: "codex".to_string(),
            instance_pid: launcher_pid,
            launcher_pid: Some(launcher_pid),
            lifecycle_phase: ClaimLifecyclePhase::WorkerSpawned,
            worker_pid: Some(worker_pid),
            claimed_at: now,
            worker_spawned_at: Some(now),
            worker_adopted_at: None,
            execution_started_at: None,
            worker_exited_at: None,
            worker_exit_detail: None,
            last_heartbeat_at: now,
        },
    )
    .unwrap();

    release_claim_after_spawned_worker_exit(
        &run_id,
        &task_id,
        worker_pid,
        "exit code 101; stderr_tail=panic",
    )
    .unwrap();

    let task = load_task(&run_id, &task_id).unwrap();
    assert_eq!(task.status, CoordinationTaskStatus::Queued);
    assert_eq!(task.claimed_by, None);

    assert!(!claim_path(&run_id, &task_id).exists());

    let events = load_events(&run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_requeued"
            && event
                .message
                .contains("Detached worker exited before task execution")
    }));
}

#[test]
fn spawned_worker_stall_detail_ignores_healthy_execution_claims() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let now = Utc::now();
    let run_id = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    let launcher_pid = 41;
    let worker_pid = 52;
    write_running_claim_fixture(
        &home,
        &run_id,
        &task_id,
        launcher_pid,
        worker_pid,
        ClaimLifecyclePhase::TaskExecutionStarted,
        now,
    );

    assert_eq!(
        spawned_worker_stall_detail(&run_id, &task_id, worker_pid, now),
        None
    );
}

#[test]
fn spawned_worker_stall_detail_flags_execution_claims_after_heartbeat_deadline() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let now = Utc::now();
    let run_id = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    let launcher_pid = 41;
    let worker_pid = 52;
    let last_heartbeat_at = now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1);
    write_running_claim_fixture(
        &home,
        &run_id,
        &task_id,
        launcher_pid,
        worker_pid,
        ClaimLifecyclePhase::TaskExecutionStarted,
        last_heartbeat_at,
    );

    let detail = spawned_worker_stall_detail(&run_id, &task_id, worker_pid, now)
        .expect("claim should be considered stale");
    assert!(detail.contains("missed the 5-minute heartbeat deadline"));
    assert!(detail.contains("Claim progress:"));
}

#[test]
fn spawned_worker_adoption_stall_detail_flags_unadopted_spawn_after_deadline() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let home = tempfile::tempdir().unwrap();
    let _home_guard = EnvVarGuard::set("HOME", home.path());

    let now = Utc::now();
    let run_id = Uuid::new_v4().to_string();
    let task_id = Uuid::new_v4().to_string();
    let launcher_pid = 41;
    let worker_pid = 52;
    let spawned_at = now - chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS + 1);

    fs::create_dir_all(tasks_dir(&run_id)).unwrap();
    fs::create_dir_all(claims_dir(&run_id)).unwrap();
    write_json_atomic(
        &run_path(&run_id),
        &CoordinationRun {
            run_id: run_id.clone(),
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
            created_at: spawned_at,
            updated_at: spawned_at,
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
            task_ids: vec![task_id.clone()],
            processed_mutation_batch_ids: BTreeSet::new(),
        },
    )
    .unwrap();
    write_json_atomic(
        &task_path(&run_id, &task_id),
        &CoordinationTask {
            task_id: task_id.clone(),
            run_id: run_id.clone(),
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
            claimed_by: Some(launcher_pid),
            created_at: spawned_at,
            updated_at: spawned_at,
            ..CoordinationTask::default()
        },
    )
    .unwrap();
    write_json_atomic(
        &claim_path(&run_id, &task_id),
        &TaskClaim {
            run_id: run_id.clone(),
            task_id: task_id.clone(),
            provider: "codex".to_string(),
            instance_pid: launcher_pid,
            launcher_pid: Some(launcher_pid),
            lifecycle_phase: ClaimLifecyclePhase::WorkerSpawned,
            worker_pid: Some(worker_pid),
            claimed_at: spawned_at,
            worker_spawned_at: Some(spawned_at),
            worker_adopted_at: None,
            execution_started_at: None,
            worker_exited_at: None,
            worker_exit_detail: None,
            last_heartbeat_at: now,
        },
    )
    .unwrap();

    let detail =
        spawned_worker_adoption_stall_detail(&run_id, &task_id, launcher_pid, worker_pid, now)
            .expect("spawned-but-unadopted claim should be considered stalled");
    assert!(detail.contains("did not adopt the launcher-owned claim"));
    assert!(detail.contains("Claim progress:"));
}

#[test]
fn spawned_worker_timeout_detail_uses_regular_task_timeout() {
    let now = Utc::now();
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: "run".to_string(),
        title: "Timed task".to_string(),
        prompt: "prompt".to_string(),
        cwd: ".".to_string(),
        kind: CoordinationTaskKind::GeneratedWriteTask,
        estimated_minutes: Some(1),
        status: CoordinationTaskStatus::Running,
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };

    let started_at = std::time::Instant::now() - std::time::Duration::from_secs(61 * 60);
    let detail = spawned_worker_timeout_detail(&task, started_at)
        .expect("regular timeout should trigger launcher watchdog detail");

    assert!(detail.contains("Worker timeout watchdog fired"));
    assert!(detail.contains("Hard wall-clock limit"));
}
