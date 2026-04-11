use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use anyhow::{Context, Result};
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
fn main() {}
