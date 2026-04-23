use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
mod restart_dialog;
use auto_runs::coordination;
use auto_runs::coordination::CoordinationEvent;
use auto_runs::coordination::CoordinationRunSnapshot;
use auto_runs::coordination::CoordinationTaskStatus;
use auto_runs::coordination::RunSummaryRow;
use auto_runs::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
use chrono::Utc;
use eframe::egui;
use restart_dialog::RestartDialogState;
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
#[derive(Default)]
struct AutoRunsApp {
    goal: String,
    workspace_path: String,
    bench_script_path: String,
    design_doc_path: String,
    lightweight_task_doc_path: String,
    lightweight_task_splitter_provider: String,
    lightweight_validation_model: String,
    lightweight_worker_models: Vec<String>,
    lightweight_repo_configuration: coordination::LightweightRepoConfiguration,
    github_branch_fix_branch: String,
    github_branch_fix_poll_minutes: u32,
    completion_criteria: String,
    notes: String,
    provider: String,
    workflow_mode: AutonomousWorkflowMode,
    execution_environment: AutonomousExecutionEnvironment,
    runsc_workspace_includes: String,
    max_iterations: u32,
    max_agents: u32,
    allow_write_tasks: bool,
    require_review_before_apply: bool,
    auto_retry_failed_tasks: bool,
    council_turn_policy: TurnPolicy,
    council_max_turns: u32,
    review_scope: ReviewScope,
    review_diff_base: String,
    review_paths: String,
    review_fix_mode: ReviewFixMode,
    runs: Vec<RunSummaryRow>,
    run_sidebar_prefixes: HashMap<String, String>,
    selected_run_id: Option<String>,
    /// All visually highlighted run IDs (single click sets anchor + this; shift-click sets range).
    selected_run_ids: HashSet<String>,
    /// Anchor run ID for shift-click range selection.
    selection_anchor_run_id: Option<String>,
    /// Last-frame selection snapshot for change detection logging.
    debug_prev_selected_ids: HashSet<String>,
    /// When true, copy the next successfully loaded selected run snapshot into the form once.
    populate_form_on_next_snapshot: bool,
    snapshot: Option<CoordinationRunSnapshot>,
    events: Vec<CoordinationEvent>,
    restart_dialog: RestartDialogState,
    errors: Vec<String>,
    selected_run_load_error: Option<String>,
    last_poll: Option<Instant>,
    last_driver_ensure_attempt: Option<Instant>,
    refresh_runs_in_flight: bool,
    selected_run_load_in_flight: Option<String>,
    selected_run_load_pending: bool,
    /// Channel to send commands to the background thread.
    cmd_tx: Option<tokio::sync::mpsc::Sender<UiThreadCmd>>,
    /// Channel to receive results from the background thread.
    result_rx: Option<Receiver<UiThreadResult>>,
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
