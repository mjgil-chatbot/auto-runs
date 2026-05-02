use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use rust_debug::tracing::{debug, info, warn};
use serde::{Deserialize, Serialize};

use crate::coordination::{self, RunSummaryRow};

pub const DEFAULT_DRIVER_POLL_INTERVAL: Duration = Duration::from_millis(500);
const DRIVER_HEARTBEAT_TTL: Duration = Duration::from_secs(3);
const DRIVER_LOCK_RECOVERY_WAIT: Duration = Duration::from_secs(5);
const DRIVER_LOCK_RECOVERY_POLL: Duration = Duration::from_millis(250);
const DRIVER_RECONCILE_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const DRIVER_POLL_MS_ENV: &str = "RUST_CHATBOT_AUTO_RUNS_DRIVER_POLL_MS";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DriverStatus {
    pub pid: u32,
    pub heartbeat_at: DateTime<Utc>,
    pub poll_interval_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub executable_path: String,
    #[serde(default)]
    pub executable_deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
}

struct DriverProcessLock(fs::File);

#[derive(Clone, Debug)]
struct DriverRuntime {
    pid: u32,
    started_at: DateTime<Utc>,
    executable_path: String,
    executable_deleted: bool,
}

impl DriverStatus {
    fn summary(&self) -> String {
        let started_at = self
            .started_at
            .map(|at| at.to_rfc3339())
            .unwrap_or_else(|| "unknown".to_string());
        format!(
            "driver pid={}, started_at={}, exe={}, deleted={}",
            self.pid, started_at, self.executable_path, self.executable_deleted
        )
    }
}

impl Drop for DriverProcessLock {
    fn drop(&mut self) {
        if let Err(err) = self.0.unlock() {
            debug!("background driver lock unlock failed: {}", err);
        }
    }
}

pub fn should_drive_run(run: &RunSummaryRow) -> bool {
    matches!(
        run.status.as_str(),
        "active" | "recovering" | "waiting_for_artifacts" | "waiting_for_policy"
    )
}

pub fn background_driver_is_healthy() -> Result<bool> {
    let Some(status) = load_background_driver_status()? else {
        return Ok(false);
    };
    Ok(driver_status_is_healthy(&status))
}

pub fn load_background_driver_status() -> Result<Option<DriverStatus>> {
    let path = driver_status_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    let status: DriverStatus = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(status))
}

pub fn drive_active_runs_once(instance_pid: u32) -> Result<bool> {
    // Read from authoritative run.json files instead of the SQLite index so a
    // stale or out-of-sync index can never hide a live run from the driver.
    let runs = coordination::query_runs_from_authoritative_files(
        &coordination::RunQuery {
            ..Default::default()
        },
        None,
    )?;
    let mut did_work = false;
    for run in runs.iter().filter(|run| should_drive_run(run)) {
        match coordination::drive_run_for_instance(&run.run_id, instance_pid, &run.provider) {
            Ok(progressed) => did_work |= progressed,
            Err(err) => {
                warn!(
                    run_id = %run.run_id,
                    provider = %run.provider,
                    status = %run.status,
                    workflow_mode = ?run.workflow_mode,
                    error = %err,
                    "background driver skipped broken run and continued"
                );
            }
        }
    }
    Ok(did_work)
}

pub fn reconcile_active_runs_once() -> Result<bool> {
    let runs = coordination::query_runs_from_authoritative_files(
        &coordination::RunQuery {
            ..Default::default()
        },
        None,
    )?;
    let mut changed = false;
    for run in runs.iter().filter(|run| should_drive_run(run)) {
        match coordination::replay_incomplete_mutation_batches_for_run(&run.run_id) {
            Ok(replayed) => changed |= replayed,
            Err(err) => warn!(run_id = %run.run_id, error = %err, "mutation replay sweep failed"),
        }
        match coordination::reconcile_run_state(&run.run_id) {
            Ok(reconciled) => changed |= reconciled,
            Err(err) => warn!(run_id = %run.run_id, error = %err, "reconcile sweep failed"),
        }
    }
    Ok(changed)
}

pub fn run_background_driver_loop() -> Result<()> {
    let Some(_lock) = acquire_driver_lock_with_recovery()? else {
        info!("background driver already active; exiting duplicate launcher");
        return Ok(());
    };

    // Ensure the worker queue FIFOs exist before entering the main loop.
    // The FIFO server runs in a spawned thread below.
    if let Err(err) = coordination::ensure_worker_queue_fifos() {
        warn!(error = %err, "failed to create worker queue FIFOs; continuing anyway");
    }

    // Spawn the FIFO worker queue server in a dedicated thread. This thread
    // lives for the lifetime of the driver process.
    thread::Builder::new()
        .name("worker-queue-server".into())
        .spawn(coordination::run_worker_queue_server)
        .with_context(|| "failed to spawn worker queue server thread")?;

    let poll_interval = poll_interval_from_env(DEFAULT_DRIVER_POLL_INTERVAL);
    let started_at = Utc::now();
    let runtime = current_driver_runtime(started_at);
    info!(
        pid = runtime.pid,
        poll_interval_ms = poll_interval.as_millis() as u64,
        executable_path = %runtime.executable_path,
        "background driver loop started"
    );
    let mut last_reconcile_sweep = Instant::now() - DRIVER_RECONCILE_SWEEP_INTERVAL;

    loop {
        let runtime = current_driver_runtime(started_at);
        if runtime.executable_deleted {
            let _ = write_driver_status(
                &runtime,
                poll_interval,
                Some("stale_executable".to_string()),
            );
            warn!(
                pid = runtime.pid,
                executable_path = %runtime.executable_path,
                "background driver executable is stale; exiting so a fresh driver can take over"
            );
            anyhow::bail!(
                "background driver executable for pid {} is stale: {}",
                runtime.pid,
                runtime.executable_path
            );
        }
        if let Err(err) = write_driver_status(&runtime, poll_interval, None) {
            warn!(error = %err, "failed to persist background driver heartbeat");
        }
        if last_reconcile_sweep.elapsed() >= DRIVER_RECONCILE_SWEEP_INTERVAL {
            if let Err(err) = reconcile_active_runs_once() {
                warn!(error = %err, "background reconciler sweep failed");
            }
            last_reconcile_sweep = Instant::now();
        }
        if let Err(err) = drive_active_runs_once(runtime.pid) {
            warn!(error = %err, "background driver iteration failed");
        }
        std::thread::sleep(poll_interval);
    }
}

pub fn current_driver_claim_context(instance_pid: u32) -> Option<String> {
    let status = load_background_driver_status().ok().flatten()?;
    (status.pid == instance_pid).then(|| status.summary())
}

fn driver_status_path() -> PathBuf {
    coordination::coordination_root_dir().join("background-driver.json")
}

fn driver_lock_path() -> PathBuf {
    coordination::coordination_root_dir().join("background-driver.lock")
}

fn write_driver_status(
    runtime: &DriverRuntime,
    poll_interval: Duration,
    stop_reason: Option<String>,
) -> Result<()> {
    let status = DriverStatus {
        pid: runtime.pid,
        heartbeat_at: Utc::now(),
        poll_interval_ms: poll_interval.as_millis().min(u128::from(u64::MAX)) as u64,
        started_at: Some(runtime.started_at),
        executable_path: runtime.executable_path.clone(),
        executable_deleted: runtime.executable_deleted,
        stop_reason,
    };
    coordination::write_json_atomic(&driver_status_path(), &status)
}

fn poll_interval_from_env(default: Duration) -> Duration {
    let Some(raw) = std::env::var_os(DRIVER_POLL_MS_ENV) else {
        return default;
    };
    let Ok(raw) = raw.into_string() else {
        return default;
    };
    let Ok(ms) = raw.parse::<u64>() else {
        return default;
    };
    if ms == 0 {
        return default;
    }
    Duration::from_millis(ms)
}

fn try_acquire_driver_lock() -> Result<Option<DriverProcessLock>> {
    let path = driver_lock_path();
    let parent = path.parent().context("driver lock path missing parent")?;
    fs::create_dir_all(parent)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(Some(DriverProcessLock(file))),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to acquire {}", path.display())),
    }
}

fn acquire_driver_lock_with_recovery() -> Result<Option<DriverProcessLock>> {
    let deadline = Instant::now() + DRIVER_LOCK_RECOVERY_WAIT;
    loop {
        if let Some(lock) = try_acquire_driver_lock()? {
            return Ok(Some(lock));
        }
        let status = load_background_driver_status()?;
        let unhealthy = status
            .as_ref()
            .map(|status| !driver_status_is_healthy(status))
            .unwrap_or(true);
        if !unhealthy || Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(DRIVER_LOCK_RECOVERY_POLL);
    }
}

fn driver_status_is_healthy(status: &DriverStatus) -> bool {
    let age = Utc::now()
        .signed_duration_since(status.heartbeat_at)
        .to_std()
        .unwrap_or_default();
    age <= DRIVER_HEARTBEAT_TTL && !status.executable_deleted && status.stop_reason.is_none()
}

fn current_driver_runtime(started_at: DateTime<Utc>) -> DriverRuntime {
    let (executable_path, executable_deleted) = current_executable_state();
    DriverRuntime {
        pid: std::process::id(),
        started_at,
        executable_path,
        executable_deleted,
    }
}

#[cfg(target_os = "linux")]
fn current_executable_state() -> (String, bool) {
    match fs::read_link("/proc/self/exe") {
        Ok(path) => {
            let path = path.to_string_lossy().into_owned();
            let deleted = path.ends_with(" (deleted)");
            (path, deleted)
        }
        Err(_) => fallback_executable_state(),
    }
}

#[cfg(not(target_os = "linux"))]
fn current_executable_state() -> (String, bool) {
    fallback_executable_state()
}

fn fallback_executable_state() -> (String, bool) {
    match std::env::current_exe() {
        Ok(path) => {
            let deleted = !path.exists();
            (path.to_string_lossy().into_owned(), deleted)
        }
        Err(_) => ("unknown".to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        driver_status_is_healthy, poll_interval_from_env, DriverStatus, DRIVER_POLL_MS_ENV,
    };
    use chrono::Utc;
    use std::time::Duration;

    #[test]
    fn poll_interval_env_ignores_invalid_values() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        unsafe {
            std::env::set_var(DRIVER_POLL_MS_ENV, "invalid");
        }
        assert_eq!(
            poll_interval_from_env(Duration::from_millis(500)),
            Duration::from_millis(500)
        );
        unsafe {
            std::env::remove_var(DRIVER_POLL_MS_ENV);
        }
    }

    #[test]
    fn poll_interval_env_uses_positive_millis() {
        let _env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        unsafe {
            std::env::set_var(DRIVER_POLL_MS_ENV, "1250");
        }
        assert_eq!(
            poll_interval_from_env(Duration::from_millis(500)),
            Duration::from_millis(1250)
        );
        unsafe {
            std::env::remove_var(DRIVER_POLL_MS_ENV);
        }
    }

    #[test]
    fn driver_status_is_unhealthy_when_executable_is_deleted() {
        let status = DriverStatus {
            pid: 41,
            heartbeat_at: Utc::now(),
            poll_interval_ms: 500,
            started_at: Some(Utc::now()),
            executable_path: "/tmp/auto-runs (deleted)".to_string(),
            executable_deleted: true,
            stop_reason: None,
        };

        assert!(!driver_status_is_healthy(&status));
    }

    #[test]
    fn driver_status_is_unhealthy_when_stop_reason_is_present() {
        let status = DriverStatus {
            pid: 41,
            heartbeat_at: Utc::now(),
            poll_interval_ms: 500,
            started_at: Some(Utc::now()),
            executable_path: "/tmp/auto-runs".to_string(),
            executable_deleted: false,
            stop_reason: Some("stale_executable".to_string()),
        };

        assert!(!driver_status_is_healthy(&status));
    }

    #[test]
    fn driver_status_is_unhealthy_when_heartbeat_is_stale() {
        let status = DriverStatus {
            pid: 41,
            heartbeat_at: Utc::now() - chrono::Duration::seconds(30),
            poll_interval_ms: 500,
            started_at: Some(Utc::now()),
            executable_path: "/tmp/auto-runs".to_string(),
            executable_deleted: false,
            stop_reason: None,
        };

        assert!(!driver_status_is_healthy(&status));
    }
}
