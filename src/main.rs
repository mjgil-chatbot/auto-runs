use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
use auto_runs::coordination::CoordinationEvent;
use auto_runs::coordination::CoordinationRunSnapshot;
use auto_runs::coordination::RunSummaryRow;
use chrono::Utc;
use eframe::egui;
use rust_debug::tracing::{debug, info, warn};
/// UI poll interval for refreshing run state from disk.
/// 500ms provides responsive UI without excessive filesystem I/O.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Commands sent from the UI thread to the background thread.
#[derive(Debug)]
enum UiThreadCmd {
    RefreshRuns,
    LoadSelectedRun(String),
    StopRun(String),
    RestartFailedTasks(String),
}
/// Results sent back from the background thread to the UI.
#[derive(Debug)]
enum UiThreadResult {
    Runs(Vec<RunSummaryRow>),
    LoadSelectedRunResult {
        run_id: String,
        snapshot: Result<CoordinationRunSnapshot, String>,
        events: Result<Vec<CoordinationEvent>, String>,
    },
    StopRunResult {
        run_id: String,
        snapshot: Result<CoordinationRunSnapshot, String>,
    },
    RestartResult {
        run_id: String,
        snapshot: Result<CoordinationRunSnapshot, String>,
    },
}
fn launch_background_driver_process() -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()
        .map_err(|err| anyhow::anyhow!("failed to resolve current executable: {err}"))?;
    let mut command = std::process::Command::new(current_exe);
    command
        .arg("drive-forever")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|err| anyhow::anyhow!("failed to spawn drive-forever: {err}"))
}
fn note_setting_value<'a>(notes: &'a str, key: &str) -> Option<&'a str> {
    notes.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = format!("{key}=");
        trimmed.strip_prefix(&prefix).and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then_some(value)
        })
    })
}
fn notes_with_setting(notes: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let existing: String = notes
        .lines()
        .filter(|line| !line.trim().starts_with(&prefix))
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    if existing.is_empty() {
        format!("{prefix}{value}")
    } else {
        format!("{existing}\n{prefix}{value}")
    }
}
fn main() {}
