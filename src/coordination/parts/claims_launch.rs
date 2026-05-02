use super::*;
#[allow(unused_imports)]
use crate::config;
#[allow(unused_imports)]
use crate::model::ModelProvider;
#[allow(unused_imports)]
use crate::AutonomousExecutionEnvironment;
#[allow(unused_imports)]
use crate::AutonomousWorkflowMode;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc, Mutex};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;
#[path = "claims_launch_runsc_mounts.rs"]
mod claims_launch_runsc_mounts;
#[allow(unused_imports)]
pub use claims_launch_runsc_mounts::autonomous_worker_runsc_mounts;
#[allow(unused_imports)]
use claims_launch_runsc_mounts::{codex_home_sidecar_dir, resolve_host_cli_binary};
#[cfg(test)]
#[path = "claims_launch_worker_failure_tests.rs"]
mod claims_launch_worker_failure_tests;
/// Hard wall-clock timeout for spawned workers: 60 minutes.
///
/// Unlike the old estimate-based timeouts, this limit is fixed and applies
/// to all task kinds. Workers that exceed this limit are treated as failed
/// and their tasks are re-queued for retry.
const WORKER_HARD_WALL_CLOCK_TIMEOUT_SECS: u64 = 60 * 60;
const LAUNCHER_CLAIM_HEARTBEAT_BRIDGE_SECS: u64 = TASK_HEARTBEAT_SECS;
fn current_executable_path() -> Result<PathBuf> {
    let path = std::env::current_exe().context("failed to resolve current executable")?;
    Ok(path.canonicalize().ok().unwrap_or(path))
}
fn expected_worker_profile(current_exe: &Path) -> &'static str {
    for ancestor in current_exe.ancestors() {
        let Some(profile_name) = ancestor.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !matches!(profile_name, "debug" | "release") {
            continue;
        }
        if ancestor
            .parent()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
            == Some("target")
        {
            return match profile_name {
                "debug" => "debug",
                "release" => "release",
                _ => unreachable!(),
            };
        }
    }
    "release"
}
fn other_profile_dir(current_dir: &Path) -> Option<(&'static str, PathBuf)> {
    let current_profile = current_dir.file_name()?.to_str()?;
    let other_profile = match current_profile {
        "debug" => "release",
        "release" => "debug",
        _ => return None,
    };
    let target_dir = current_dir.parent()?;
    if target_dir.file_name().and_then(|name| name.to_str()) != Some("target") {
        return None;
    }
    Some((other_profile, target_dir.join(other_profile)))
}
fn resolve_companion_binary_from_paths(
    current_exe: &Path,
    manifest_dir: &Path,
    profile: &str,
    binary_name: &str,
) -> Result<PathBuf> {
    let current_dir = current_exe.parent().unwrap_or(Path::new("."));
    let worker_in_profile = current_dir.join(binary_name);
    if worker_in_profile.exists() {
        return Ok(worker_in_profile);
    }

    if let Some((other_profile, other_dir)) = other_profile_dir(current_dir) {
        let mismatch = other_dir.join(binary_name);
        if mismatch.exists() {
            let current_profile = current_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("current");
            anyhow::bail!(
                "{} was not found in the current {} build directory {} for {}. Found only {} in {} instead; mixed debug/release launches are not allowed.",
                binary_name,
                current_profile,
                current_dir.display(),
                current_exe.display(),
                mismatch.display(),
                other_profile,
            );
        }
    }

    let manifest_dir = manifest_dir
        .canonicalize()
        .ok()
        .unwrap_or_else(|| manifest_dir.to_path_buf());
    let repo_target = manifest_dir.join("target").join(profile).join(binary_name);
    if repo_target.exists() {
        return Ok(repo_target.canonicalize().ok().unwrap_or(repo_target));
    }

    let other_profile = match profile {
        "debug" => Some("release"),
        "release" => Some("debug"),
        _ => None,
    };
    if let Some(other_profile) = other_profile {
        let mismatch = manifest_dir
            .join("target")
            .join(other_profile)
            .join(binary_name);
        if mismatch.exists() {
            anyhow::bail!(
                "{} was not found in the expected {} build directory {} for {}. Found only {} instead; mixed debug/release launches are not allowed.",
                binary_name,
                profile,
                repo_target.parent().unwrap_or(Path::new(".")).display(),
                current_exe.display(),
                mismatch.display(),
            );
        }
    }

    anyhow::bail!(
        "{} was not found alongside the current executable {} in {} or in the repo-local {} build directory {}. Build {} in the same profile first.",
        binary_name,
        current_exe.display(),
        current_dir.display(),
        profile,
        repo_target.parent().unwrap_or(Path::new(".")).display(),
        binary_name,
    );
}
pub fn resolve_autonomous_worker_binary() -> Result<PathBuf> {
    let current_exe = current_executable_path()?;
    resolve_companion_binary_from_paths(
        &current_exe,
        Path::new(env!("CARGO_MANIFEST_DIR")),
        expected_worker_profile(&current_exe),
        "auto-runs-worker",
    )
}
pub fn find_setsid_binary() -> Option<PathBuf> {
    for candidate in &["/usr/bin/setsid", "/bin/setsid", "/usr/local/bin/setsid"] {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }
    None
}
fn worker_logs_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("worker-logs")
}
fn worker_stderr_log_path(run_id: &str, task_id: &str) -> PathBuf {
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.fZ");
    worker_logs_dir(run_id).join(format!("{task_id}.{stamp}.stderr.log"))
}
fn stderr_stdio_for_log(path: Option<&Path>) -> Result<std::process::Stdio> {
    let Some(path) = path else {
        return Ok(std::process::Stdio::null());
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    Ok(std::process::Stdio::from(file))
}
fn read_worker_stderr_excerpt(path: &Path) -> Option<String> {
    let raw = fs::read_to_string(path).ok()?;
    let collapsed = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if collapsed.is_empty() {
        return None;
    }

    const MAX_CHARS: usize = 400;
    let mut excerpt = collapsed.chars().take(MAX_CHARS).collect::<String>();
    if collapsed.chars().count() > MAX_CHARS {
        excerpt.push_str("...");
    }
    Some(excerpt)
}
fn worker_exit_detail(detail: &str, stderr_log_path: &Path) -> String {
    let mut decorated = format!("{detail}; stderr_log={}", stderr_log_path.display());
    if let Some(excerpt) = read_worker_stderr_excerpt(stderr_log_path) {
        decorated.push_str(&format!("; stderr_tail={excerpt}"));
    }
    decorated
}
const SPAWNED_WORKER_EXIT_POLL_MS: u64 = 250;
fn autonomous_worker_spawn_args(
    task: &CoordinationTask,
    provider_name: &str,
    launcher_pid: u32,
) -> [String; 5] {
    [
        "autonomous".to_string(),
        task.run_id.clone(),
        task.task_id.clone(),
        provider_name.to_string(),
        launcher_pid.to_string(),
    ]
}
fn terminate_spawned_worker_process(child: &mut std::process::Child) {
    let pid = child.id() as libc::pid_t;
    if pid <= 0 {
        let _ = child.kill();
        return;
    }

    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    for _ in 0..10 {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break,
        }
    }
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.kill();
}
fn spawned_worker_stall_detail(
    run_id: &str,
    task_id: &str,
    worker_pid: u32,
    now: DateTime<Utc>,
) -> Option<String> {
    let claim = load_claim(run_id, task_id).ok()?;
    if claim.instance_pid != worker_pid {
        return None;
    }
    if claim.lifecycle_phase != ClaimLifecyclePhase::TaskExecutionStarted {
        return None;
    }

    let stale_after = claim_heartbeat_stale_at(&claim);
    if stale_after > now {
        return None;
    }

    let stalled_for = now
        .signed_duration_since(claim.last_heartbeat_at)
        .num_seconds()
        .max(0);
    Some(format!(
        "Worker PID {worker_pid} missed the 5-minute heartbeat deadline after {stalled_for} seconds without a heartbeat. Claim progress: {}.",
        claim_lifecycle_summary(&claim)
    ))
}
fn spawned_worker_adoption_stall_detail(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    worker_pid: u32,
    now: DateTime<Utc>,
) -> Option<String> {
    let claim = load_claim(run_id, task_id).ok()?;
    if claim.instance_pid != launcher_pid || claim.worker_pid != Some(worker_pid) {
        return None;
    }
    if claim.lifecycle_phase != ClaimLifecyclePhase::WorkerSpawned {
        return None;
    }

    let spawned_at = claim.worker_spawned_at.unwrap_or(claim.claimed_at);
    let stale_after = spawned_at + chrono::Duration::seconds(TASK_HEARTBEAT_STALE_SECS);
    if stale_after > now {
        return None;
    }

    Some(format!(
        "Worker PID {worker_pid} did not adopt the launcher-owned claim within the 5-minute startup window. Claim progress: {}.",
        claim_lifecycle_summary(&claim)
    ))
}
fn spawned_worker_timeout_detail(
    task: &CoordinationTask,
    started_at: std::time::Instant,
) -> Option<String> {
    if started_at.elapsed().as_secs() < WORKER_HARD_WALL_CLOCK_TIMEOUT_SECS {
        return None;
    }
    let timeout = Duration::from_secs(WORKER_HARD_WALL_CLOCK_TIMEOUT_SECS);
    Some(format!(
        "Worker timeout watchdog fired for {} after {} minutes. Hard wall-clock limit reached.",
        task.title,
        timeout.as_secs() / 60
    ))
}
fn release_claim_after_spawned_worker_exit(
    run_id: &str,
    task_id: &str,
    worker_pid: u32,
    detail: &str,
) -> Result<()> {
    let claim = match load_claim(run_id, task_id) {
        Ok(claim) => claim,
        Err(_) => return Ok(()),
    };

    let err = anyhow::anyhow!(
        "Detached worker PID {} exited before reaching a terminal artifact. {}",
        worker_pid,
        detail
    );

    if claim.instance_pid == worker_pid {
        return release_claim_after_worker_failure(run_id, task_id, worker_pid, &err);
    }

    if claim.worker_pid != Some(worker_pid) {
        return Ok(());
    }

    let now = Utc::now();
    let mut task = load_task(run_id, task_id)?;
    if !matches!(
        task.status,
        CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
    ) {
        let _ = fs::remove_file(claim_path(run_id, task_id));
        return Ok(());
    }
    task.claimed_by = None;
    task.updated_at = now;

    if task.attempt_count >= task.max_attempts.max(1) {
        task.status = CoordinationTaskStatus::Failed;
        task.artifact_summary = Some(err.to_string());
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        append_event(
            run_id,
            "task_failed",
            Some(task_id),
            &format!(
                "Detached worker exited before task execution for {}. Marked as failed after {}/{} attempts. {}",
                task.title,
                task.attempt_count,
                task.max_attempts.max(1),
                err
            ),
        )?;
        append_event(
            run_id,
            "task_exhausted_attempts",
            Some(task_id),
            &format!(
                "Detached worker exited before task execution for {}. Marked as failed after {}/{} attempts. {}",
                task.title,
                task.attempt_count,
                task.max_attempts.max(1),
                err
            ),
        )?;
    } else {
        task.status = CoordinationTaskStatus::Queued;
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        append_event(
            run_id,
            "task_requeued",
            Some(task_id),
            &format!(
                "Detached worker exited before task execution for {}. Returned task to queue (attempt {}/{}). {}",
                task.title,
                task.attempt_count,
                task.max_attempts.max(1),
                err
            ),
        )?;
    }
    sync_run_summary_after_task_update(run_id, task.updated_at)?;
    Ok(())
}
pub fn spawn_daemonized(
    target: &Path,
    args: &[&str],
    env_vars: &[(&str, String)],
) -> Result<std::process::Child> {
    if let Some(setsid_bin) = find_setsid_binary() {
        let mut cmd = std::process::Command::new(&setsid_bin);
        cmd.arg("--");
        cmd.arg(target);
        for arg in args {
            cmd.arg(arg);
        }
        for (key, val) in env_vars {
            cmd.env(key, val);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn setsid daemonizer")?;
        let _ = child.try_wait().ok();
        return Ok(child);
    }

    let mut cmd = std::process::Command::new(target);
    for arg in args {
        cmd.arg(arg);
    }
    for (key, val) in env_vars {
        cmd.env(key, val);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .with_context(|| format!("failed to spawn {}", target.display()))
}
pub fn spawn_provider_task_process(
    task: &CoordinationTask,
    provider: &str,
    launcher_pid: u32,
) -> Result<()> {
    let run = load_run(&task.run_id)?;
    let worker_bin = resolve_autonomous_worker_binary()?;
    let provider_name = task.assigned_provider.as_deref().unwrap_or(provider);
    let worker_args = autonomous_worker_spawn_args(task, provider_name, launcher_pid);
    let args = worker_args.iter().map(String::as_str).collect::<Vec<_>>();

    let mut env_vars: Vec<(&str, String)> = vec![];
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        let effective_includes = effective_runsc_workspace_includes(&run, Some(task));
        env_vars.push(("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV", "runsc".to_string()));
        env_vars.push((
            "RUST_CHATBOT_AUTONOMOUS_RUNSC_WORKSPACE_INCLUDES",
            serde_json::to_string(&effective_includes).unwrap_or_else(|_| "[]".to_string()),
        ));
    }

    let stderr_log_path = worker_stderr_log_path(&task.run_id, &task.task_id);
    let mut child = spawn_daemonized_with_stderr(&worker_bin, &args, &env_vars, &stderr_log_path)
        .with_context(|| {
        format!(
            "failed to spawn daemonized auto-runs-worker for run {} task {}",
            task.run_id, task.task_id
        )
    })?;
    let worker_pid = child.id();
    let _ = record_worker_spawned(&task.run_id, &task.task_id, launcher_pid, worker_pid);

    let task_run_id = task.run_id.clone();
    let task_id = task.task_id.clone();
    let launcher_pid_for_wait = launcher_pid;
    let stderr_log_path_for_wait = stderr_log_path.clone();
    let forced_exit_detail = Arc::new(Mutex::new(None::<String>));
    let forced_exit_detail_for_wait = Arc::clone(&forced_exit_detail);
    let watchdog_task = task.clone();
    let started_at = std::time::Instant::now();
    let mut next_launcher_heartbeat =
        std::time::Instant::now() + Duration::from_secs(LAUNCHER_CLAIM_HEARTBEAT_BRIDGE_SECS);
    thread::spawn(move || loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    let raw_detail = forced_exit_detail_for_wait
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .take()
                        .unwrap_or_else(|| format_worker_exit_status(status));
                    let detail = worker_exit_detail(&raw_detail, &stderr_log_path_for_wait);
                    let _ = record_worker_exited(&task_run_id, &task_id, worker_pid, &detail);
                    let _ = release_claim_after_spawned_worker_exit(
                        &task_run_id,
                        &task_id,
                        worker_pid,
                        &detail,
                    );
                    debug!(
                        run_id = %task_run_id,
                        task_id = %task_id,
                        status = %detail,
                        "spawned worker process exited with non-zero status"
                    );
                }
                return;
            }
            Ok(None) => {
                if std::time::Instant::now() >= next_launcher_heartbeat {
                    let _ = heartbeat_claim(&task_run_id, &task_id, launcher_pid_for_wait);
                    next_launcher_heartbeat = std::time::Instant::now()
                        + Duration::from_secs(LAUNCHER_CLAIM_HEARTBEAT_BRIDGE_SECS);
                }
                let mut forced_detail = forced_exit_detail_for_wait
                    .lock()
                    .unwrap_or_else(|err| err.into_inner());
                if forced_detail.is_none() {
                    let now = Utc::now();
                    if let Some(detail) = spawned_worker_adoption_stall_detail(
                        &task_run_id,
                        &task_id,
                        launcher_pid_for_wait,
                        worker_pid,
                        now,
                    ) {
                        *forced_detail = Some(detail);
                        terminate_spawned_worker_process(&mut child);
                    } else if let Some(detail) =
                        spawned_worker_stall_detail(&task_run_id, &task_id, worker_pid, now)
                    {
                        *forced_detail = Some(detail);
                        terminate_spawned_worker_process(&mut child);
                    } else if let Some(detail) =
                        spawned_worker_timeout_detail(&watchdog_task, started_at)
                    {
                        *forced_detail = Some(detail);
                        terminate_spawned_worker_process(&mut child);
                    }
                }
            }
            Err(err) => {
                warn!(
                    run_id = %task_run_id,
                    task_id = %task_id,
                    error = %err,
                    "failed to wait on spawned worker process"
                );
                return;
            }
        }
        thread::sleep(Duration::from_millis(SPAWNED_WORKER_EXIT_POLL_MS));
    });
    Ok(())
}
pub fn spawn_daemonized_with_stderr(
    target: &Path,
    args: &[&str],
    env_vars: &[(&str, String)],
    stderr_log_path: &Path,
) -> Result<std::process::Child> {
    if let Some(setsid_bin) = find_setsid_binary() {
        let mut cmd = std::process::Command::new(&setsid_bin);
        cmd.arg("--");
        cmd.arg(target);
        for arg in args {
            cmd.arg(arg);
        }
        for (key, val) in env_vars {
            cmd.env(key, val);
        }
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(stderr_stdio_for_log(Some(stderr_log_path))?);
        let mut child = cmd
            .spawn()
            .with_context(|| "failed to spawn setsid daemonizer")?;
        let _ = child.try_wait().ok();
        return Ok(child);
    }

    let mut cmd = std::process::Command::new(target);
    for arg in args {
        cmd.arg(arg);
    }
    for (key, val) in env_vars {
        cmd.env(key, val);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr_stdio_for_log(Some(stderr_log_path))?);
    cmd.spawn()
        .with_context(|| format!("failed to spawn {}", target.display()))
}
pub fn handle_launch_failure_after_claim(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    err: &anyhow::Error,
) -> Result<()> {
    finalize_launch_failure(run_id, task_id, launcher_pid, err, false)
}
pub fn release_claim_after_launch_failure(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    err: &anyhow::Error,
) -> Result<()> {
    finalize_launch_failure(run_id, task_id, launcher_pid, err, true)
}
pub fn release_claim_after_worker_failure(
    run_id: &str,
    task_id: &str,
    worker_pid: u32,
    err: &anyhow::Error,
) -> Result<()> {
    let now = Utc::now();
    let mut task = load_task(run_id, task_id)?;
    let claim_owned_by_worker = load_claim(run_id, task_id)
        .map(|claim| claim.instance_pid == worker_pid)
        .unwrap_or_else(|_| task.claimed_by == Some(worker_pid));
    if !claim_owned_by_worker {
        return Ok(());
    }
    if matches!(
        task.status,
        CoordinationTaskStatus::Running | CoordinationTaskStatus::Claiming
    ) {
        let exhausted = task.attempt_count >= task.max_attempts.max(1);
        task.status = if exhausted {
            CoordinationTaskStatus::Failed
        } else {
            CoordinationTaskStatus::Queued
        };
        task.claimed_by = None;
        task.updated_at = now;
        if exhausted && task.artifact_summary.is_none() {
            task.artifact_summary = Some(err.to_string());
        }
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        append_event(
            run_id,
            if exhausted {
                "task_exhausted_attempts"
            } else {
                "task_requeued"
            },
            Some(task_id),
            &format!(
                "Worker failed after claim adoption for {} (attempt {}/{}). {}",
                task.title,
                task.attempt_count,
                task.max_attempts.max(1),
                err
            ),
        )?;
        sync_run_summary_after_task_update(run_id, task.updated_at)?;
        return Ok(());
    }

    let _ = fs::remove_file(claim_path(run_id, task_id));
    Ok(())
}
pub fn finalize_launch_failure(
    run_id: &str,
    task_id: &str,
    launcher_pid: u32,
    err: &anyhow::Error,
    increment_attempt: bool,
) -> Result<()> {
    let mut task = load_task(run_id, task_id)?;
    let claim_owned_by_launcher = load_claim(run_id, task_id)
        .map(|claim| claim.instance_pid == launcher_pid)
        .unwrap_or_else(|_| task.claimed_by == Some(launcher_pid));
    if !claim_owned_by_launcher {
        return Ok(());
    }

    if increment_attempt {
        task.attempt_count = task.attempt_count.saturating_add(1);
    }
    task.claimed_by = None;
    task.updated_at = Utc::now();

    if task.attempt_count >= task.max_attempts.max(1) {
        task.status = CoordinationTaskStatus::Failed;
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        let msg = format!(
            "Worker launch failed for {}. Marked as failed after {}/{} attempts. {}",
            task.title,
            task.attempt_count,
            task.max_attempts.max(1),
            err
        );
        append_event(run_id, "task_failed", Some(task_id), &msg)?;
        append_event(run_id, "task_exhausted_attempts", Some(task_id), &msg)?;
    } else {
        task.status = CoordinationTaskStatus::Queued;
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        append_event(
            run_id,
            "task_requeued",
            Some(task_id),
            &format!(
                "Worker launch failed for {}. Returned task to queue (attempt {}/{}). {}",
                task.title,
                task.attempt_count,
                task.max_attempts.max(1),
                err
            ),
        )?;
    }
    Ok(())
}
