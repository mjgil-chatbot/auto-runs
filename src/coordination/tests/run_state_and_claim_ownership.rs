fn make_claim_for_test(run_id: &str, task_id: &str, instance_pid: u32, launcher_pid: u32) -> TaskClaim {
    let now = Utc::now();
    TaskClaim {
        run_id: run_id.to_string(),
        task_id: task_id.to_string(),
        provider: "codex".to_string(),
        instance_pid,
        launcher_pid: Some(launcher_pid),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: now,
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: now,
    }
}
