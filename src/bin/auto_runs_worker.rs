use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use auto_runs::coordination::{
    adopt_claim_for_worker, poll_task_until_found, record_worker_adopt_failed,
    record_worker_adopted, record_worker_started, release_claim_after_launch_failure,
    release_claim_after_worker_failure, run_provider_task_for_claim,
};
use rust_debug::tracing::{info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorkerArgs {
    run_id: String,
    task_id: String,
    provider: String,
    launcher_pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PollWorkerArgs {
    run_id: String,
    provider: String,
}

fn usage() -> &'static str {
    "Usage: auto-runs-worker autonomous <run_id> <task_id> <provider> <launcher_pid>\n\
     or:  auto-runs-worker autonomous-poll <run_id> <provider>"
}

fn parse_args_from<I>(args: I) -> Result<WorkerArgs>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if args.len() != 6 || args.get(1).map(String::as_str) != Some("autonomous") {
        anyhow::bail!(usage());
    }

    let launcher_pid = args[5]
        .parse::<u32>()
        .with_context(|| format!("invalid launcher pid `{}`", args[5]))?;

    Ok(WorkerArgs {
        run_id: args[2].clone(),
        task_id: args[3].clone(),
        provider: args[4].clone(),
        launcher_pid,
    })
}

fn parse_poll_args_from<I>(args: I) -> Result<PollWorkerArgs>
where
    I: IntoIterator<Item = String>,
{
    let args: Vec<String> = args.into_iter().collect();
    if args.len() != 4 || args.get(1).map(String::as_str) != Some("autonomous-poll") {
        anyhow::bail!(usage());
    }

    Ok(PollWorkerArgs {
        run_id: args[2].clone(),
        provider: args[3].clone(),
    })
}

fn run_autonomous_worker(args: &WorkerArgs) -> Result<()> {
    let worker_pid = std::process::id();
    info!(
        run_id = %args.run_id,
        task_id = %args.task_id,
        provider = %args.provider,
        launcher_pid = args.launcher_pid,
        worker_pid,
        "auto-runs worker starting"
    );

    run_provider_task_for_claim(
        &args.run_id,
        &args.task_id,
        &args.provider,
        worker_pid,
        Arc::new(AtomicBool::new(false)),
    )?;
    Ok(())
}

fn run_poll_worker(args: &PollWorkerArgs) -> Result<()> {
    let worker_pid = std::process::id();
    info!(
        run_id = %args.run_id,
        provider = %args.provider,
        worker_pid,
        "auto-runs worker starting in poll mode"
    );

    // Poll the FIFO server until we get a task assignment or timeout.
    let Some(assignment) = poll_task_until_found(&args.run_id, worker_pid, &args.provider) else {
        anyhow::bail!(
            "poll timeout: no task available for run {} provider {} within 1 hour",
            args.run_id,
            args.provider
        );
    };

    let task_id = assignment.task_id;
    info!(
        run_id = %args.run_id,
        task_id = %task_id,
        provider = %args.provider,
        worker_pid,
        "worker received task assignment via poll"
    );

    // Record worker start (using 0 as launcher_pid since there's no coordinator launch in poll mode).
    if let Err(err) = record_worker_started(&args.run_id, &task_id, &args.provider, 0, worker_pid) {
        warn!(
            run_id = %args.run_id,
            task_id = %task_id,
            error = %err,
            "failed to record worker start event"
        );
    }

    // Adopt the claim that we just received via the FIFO.
    let adopted = match adopt_claim_for_worker(&args.run_id, &task_id, 0, worker_pid) {
        Ok(()) => {
            if let Err(err) = record_worker_adopted(&args.run_id, &task_id, worker_pid) {
                warn!(
                    run_id = %args.run_id,
                    task_id = %task_id,
                    error = %err,
                    "failed to record worker adoption event"
                );
            }
            true
        }
        Err(err) => {
            let err = err.context("failed to adopt task assignment from FIFO");
            warn!(
                run_id = %args.run_id,
                task_id = %task_id,
                provider = %args.provider,
                error = %err,
                "worker failed to adopt assigned task"
            );
            if let Err(record_err) = record_worker_adopt_failed(
                &args.run_id,
                &task_id,
                &args.provider,
                0,
                worker_pid,
                &err.to_string(),
            ) {
                warn!(
                    run_id = %args.run_id,
                    task_id = %task_id,
                    error = %record_err,
                    "failed to record worker adoption failure"
                );
            }
            eprintln!("{err}");
            return Err(err);
        }
    };

    // Execute the task.
    let run_result = run_provider_task_for_claim(
        &args.run_id,
        &task_id,
        &args.provider,
        worker_pid,
        Arc::new(AtomicBool::new(false)),
    );

    if let Err(err) = run_result {
        warn!(
            run_id = %args.run_id,
            task_id = %task_id,
            provider = %args.provider,
            error = %err,
            "worker task execution failed"
        );
        let release_result = if adopted {
            release_claim_after_worker_failure(&args.run_id, &task_id, worker_pid, &err)
        } else {
            release_claim_after_launch_failure(&args.run_id, &task_id, 0, &err)
        };
        if let Err(release_err) = release_result {
            warn!(
                run_id = %args.run_id,
                task_id = %task_id,
                error = %release_err,
                "failed to release claim after task failure"
            );
        }
        eprintln!("{err}");
        return Err(err);
    }

    Ok(())
}

fn main() -> ExitCode {
    auto_runs::process_limits::apply_default_process_memory_limit("auto-runs-worker");
    let _debug_guard = chatbot_lib::worker_process::init_resilient_tracing("auto-runs-worker");
    rust_debug::install_panic_hook();
    auto_runs::ensure_path_symlink();

    // Try poll mode first (pull-based worker).
    if let Ok(args) = parse_poll_args_from(std::env::args()) {
        if let Err(err) = run_poll_worker(&args) {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // Fall back to push mode (coordinator-assigned task).
    let args = match parse_args_from(std::env::args()) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    let worker_pid = std::process::id();
    if let Err(err) = record_worker_started(
        &args.run_id,
        &args.task_id,
        &args.provider,
        args.launcher_pid,
        worker_pid,
    ) {
        warn!(
            run_id = %args.run_id,
            task_id = %args.task_id,
            error = %err,
            "failed to record worker start event"
        );
    }
    let adopted_claim = match adopt_claim_for_worker(
        &args.run_id,
        &args.task_id,
        args.launcher_pid,
        worker_pid,
    ) {
        Ok(()) => {
            if let Err(err) = record_worker_adopted(&args.run_id, &args.task_id, worker_pid) {
                warn!(
                    run_id = %args.run_id,
                    task_id = %args.task_id,
                    error = %err,
                    "failed to record worker adoption event"
                );
            }
            true
        }
        Err(err) => {
            let err = err.context("failed to adopt claimed task for detached worker");
            warn!(
                run_id = %args.run_id,
                task_id = %args.task_id,
                provider = %args.provider,
                launcher_pid = args.launcher_pid,
                error = %err,
                "auto-runs worker failed"
            );
            if let Err(record_err) = record_worker_adopt_failed(
                &args.run_id,
                &args.task_id,
                &args.provider,
                args.launcher_pid,
                worker_pid,
                &err.to_string(),
            ) {
                warn!(
                    run_id = %args.run_id,
                    task_id = %args.task_id,
                    error = %record_err,
                    "failed to record worker adoption failure"
                );
            }
            if let Err(release_err) = release_claim_after_launch_failure(
                &args.run_id,
                &args.task_id,
                args.launcher_pid,
                &err,
            ) {
                warn!(
                    run_id = %args.run_id,
                    task_id = %args.task_id,
                    error = %release_err,
                    "failed to release claim after launch failure — claim may remain until stale-claim cleanup"
                );
            }
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(err) = run_autonomous_worker(&args) {
        warn!(
            run_id = %args.run_id,
            task_id = %args.task_id,
            provider = %args.provider,
            launcher_pid = args.launcher_pid,
            error = %err,
            "auto-runs worker failed"
        );
        let release_result = if adopted_claim {
            release_claim_after_worker_failure(&args.run_id, &args.task_id, worker_pid, &err)
        } else {
            release_claim_after_launch_failure(&args.run_id, &args.task_id, args.launcher_pid, &err)
        };
        if let Err(release_err) = release_result {
            warn!(
                run_id = %args.run_id,
                task_id = %args.task_id,
                error = %release_err,
                "failed to release claim after launch failure — claim may remain until stale-claim cleanup"
            );
        }
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{parse_args_from, usage, WorkerArgs};

    #[test]
    fn parse_args_accepts_autonomous_invocation() {
        let args = parse_args_from([
            "auto-runs-worker".to_string(),
            "autonomous".to_string(),
            "run-1".to_string(),
            "task-1".to_string(),
            "codex".to_string(),
            "42".to_string(),
        ])
        .unwrap();

        assert_eq!(
            args,
            WorkerArgs {
                run_id: "run-1".to_string(),
                task_id: "task-1".to_string(),
                provider: "codex".to_string(),
                launcher_pid: 42,
            }
        );
    }

    #[test]
    fn parse_args_rejects_terminal_worker_shape() {
        let err =
            parse_args_from(["auto-runs-worker".to_string(), "chat-id".to_string()]).unwrap_err();

        assert!(err.to_string().contains(usage()));
    }
}
