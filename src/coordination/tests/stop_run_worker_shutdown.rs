#[test]
fn stop_run_terminates_adopted_worker_processes() {
    let fixture = CoordinationTestFixture::named("stop-run-worker-shutdown");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut running = fixture.make_task(
        &run,
        "running",
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Running task",
        &["src/lib.rs"],
        &["src/lib.rs"],
        &[],
        CoordinationTaskStatus::Running,
    );
    let mut child = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("sleep process should spawn");
    let worker_pid = child.id();
    running.claimed_by = Some(worker_pid);
    fixture.persist_run_with_tasks(&run, &[running.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: running.task_id.clone(),
        provider: "codex".to_string(),
        instance_pid: worker_pid,
        launcher_pid: Some(1234),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(worker_pid),
        claimed_at: Utc::now(),
        worker_spawned_at: Some(Utc::now()),
        worker_adopted_at: Some(Utc::now()),
        execution_started_at: Some(Utc::now()),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    let snapshot = stop_run(&run.run_id, "user requested stop").expect("stop should succeed");

    assert_eq!(snapshot.run.status, CoordinationRunStatus::Canceled);
    assert!(
        child
            .wait_timeout(std::time::Duration::from_secs(2))
            .expect("wait on child should succeed")
            .is_some(),
        "adopted worker process should exit during stop"
    );
}

trait ChildWaitTimeout {
    fn wait_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl ChildWaitTimeout for std::process::Child {
    fn wait_timeout(
        &mut self,
        timeout: std::time::Duration,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        let start = std::time::Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(Some(status));
            }
            if start.elapsed() >= timeout {
                return Ok(None);
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
}
