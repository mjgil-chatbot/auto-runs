use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;
mod restart_dialog;
mod run_form;
use auto_runs::coordination;
use auto_runs::coordination::CoordinationEvent;
use auto_runs::coordination::CoordinationRunSnapshot;
use auto_runs::coordination::CoordinationTaskStatus;
use auto_runs::coordination::RunSummaryRow;
use auto_runs::driver;
use auto_runs::model::ModelProvider;
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
impl AutoRunsApp {
    fn ensure_background_driver(&mut self) {
        let should_attempt = self
            .last_driver_ensure_attempt
            .is_none_or(|last_attempt| last_attempt.elapsed() >= Duration::from_secs(5));
        if !should_attempt {
            return;
        }
        self.last_driver_ensure_attempt = Some(Instant::now());

        match driver::background_driver_is_healthy() {
            Ok(true) => {}
            Ok(false) => {
                if let Err(err) = launch_background_driver_process() {
                    self.append_error(format!("failed to relaunch background driver: {err}"));
                }
            }
            Err(err) => {
                self.append_error(format!("failed to inspect background driver health: {err}"));
            }
        }
    }}
impl AutoRunsApp {
    fn append_error(&mut self, error: impl Into<String>) {
        let message = error.into();
        warn!(error = %message, "auto-runs ui action failed");
        self.errors.push(message);
        if self.errors.len() > 8 {
            self.errors.remove(0);
        }
    }}
impl AutoRunsApp {
    fn ensure_defaults(&mut self) {
        if self.workspace_path.is_empty() {
            self.workspace_path = std::env::current_dir()
                .ok()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if self.provider.is_empty() {
            self.provider = ModelProvider::Codex.as_str().to_string();
        }
        if self.lightweight_task_splitter_provider.is_empty() {
            self.lightweight_task_splitter_provider = ModelProvider::Codex.as_str().to_string();
        }
        if self.lightweight_validation_model.is_empty() {
            self.lightweight_validation_model = self.lightweight_task_splitter_provider.clone();
        }
        if self
            .lightweight_worker_models
            .iter()
            .all(|value| value.trim().is_empty())
        {
            self.lightweight_worker_models = vec![ModelProvider::Codex.as_str().to_string()];
        }
        if self.github_branch_fix_poll_minutes == 0 {
            self.github_branch_fix_poll_minutes = 5;
        }
        if self.max_iterations == 0 {
            self.max_iterations = 10;
        }
        if self.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
            self.max_agents = 1;
        } else if self.max_agents == 0 {
            self.max_agents = 4;
        }
        if self.council_max_turns == 0 {
            self.council_max_turns = 40;
        }
        if !self.allow_write_tasks && !self.require_review_before_apply && self.goal.is_empty() {
            self.allow_write_tasks = true;
            self.require_review_before_apply = true;
            self.auto_retry_failed_tasks = true;
        }
    }}
impl AutoRunsApp {
    fn set_workflow_mode(&mut self, mode: AutonomousWorkflowMode) {
        let should_reset_design_doc_path = self.design_doc_path.trim().is_empty()
            || auto_runs::is_generated_design_doc_path(&self.design_doc_path);
        self.workflow_mode = mode;
        self.auto_retry_failed_tasks = true;
        if should_reset_design_doc_path {
            self.design_doc_path =
                auto_runs::default_design_doc_path_for_workflow(&self.workflow_mode);
            if coordination::uses_lightweight_runtime_mode(&self.workflow_mode) {
                self.lightweight_task_doc_path =
                    coordination::lightweight_default_task_doc_path(&self.design_doc_path);
            }
        }
        if matches!(
            self.workflow_mode,
            AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
        ) {
            self.max_agents = 1;
        } else if self.max_agents == 0 {
            self.max_agents = 4;
        }
        if self.workflow_mode != AutonomousWorkflowMode::CodeReview {
            self.review_scope = ReviewScope::Worktree;
            self.review_diff_base.clear();
            self.review_paths.clear();
            self.review_fix_mode = ReviewFixMode::default();
        }
        if self.workflow_mode != AutonomousWorkflowMode::FreeformCouncil {
            self.council_turn_policy = TurnPolicy::default();
            self.council_max_turns = 40;
        }
        if !coordination::uses_lightweight_runtime_mode(&self.workflow_mode) {
            self.lightweight_task_doc_path.clear();
            self.lightweight_task_splitter_provider.clear();
            self.lightweight_validation_model.clear();
            self.lightweight_worker_models.clear();
            self.lightweight_repo_configuration =
                coordination::LightweightRepoConfiguration::default();
            self.github_branch_fix_branch.clear();
            self.github_branch_fix_poll_minutes = 0;
        }
    }}
impl AutoRunsApp {
    fn set_execution_environment(&mut self, environment: AutonomousExecutionEnvironment) {
        self.execution_environment = environment;
        if self.execution_environment == AutonomousExecutionEnvironment::Runsc
            && self.runsc_workspace_includes.trim().is_empty()
        {
            self.runsc_workspace_includes = ".\n".to_string();
        }
    }}
impl AutoRunsApp {
    fn start_new_run(&mut self) {
        self.goal.clear();
        self.workspace_path.clear();
        self.bench_script_path.clear();
        self.design_doc_path.clear();
        self.lightweight_task_doc_path.clear();
        self.lightweight_task_splitter_provider.clear();
        self.lightweight_validation_model.clear();
        self.lightweight_worker_models.clear();
        self.lightweight_repo_configuration = coordination::LightweightRepoConfiguration::default();
        self.github_branch_fix_branch.clear();
        self.github_branch_fix_poll_minutes = 0;
        self.completion_criteria.clear();
        self.notes.clear();
        self.provider.clear();
        self.workflow_mode = AutonomousWorkflowMode::default();
        self.execution_environment = AutonomousExecutionEnvironment::default();
        self.runsc_workspace_includes.clear();
        self.max_iterations = 0;
        self.max_agents = 0;
        self.allow_write_tasks = false;
        self.require_review_before_apply = false;
        self.auto_retry_failed_tasks = false;
        self.council_turn_policy = TurnPolicy::default();
        self.council_max_turns = 0;
        self.review_scope = ReviewScope::default();
        self.review_diff_base.clear();
        self.review_paths.clear();
        self.review_fix_mode = ReviewFixMode::default();
        self.selected_run_id = None;
        self.selected_run_ids.clear();
        self.selection_anchor_run_id = None;
        self.debug_prev_selected_ids.clear();
        self.populate_form_on_next_snapshot = false;
        self.refresh_runs_in_flight = false;
        self.selected_run_load_in_flight = None;
        self.selected_run_load_pending = false;
        self.snapshot = None;
        self.events.clear();
        self.selected_run_load_error = None;
        self.restart_dialog = RestartDialogState::default();
        self.ensure_defaults();
        self.set_workflow_mode(AutonomousWorkflowMode::default());
    }}
impl AutoRunsApp {
    fn refresh_runs(&mut self) {
        match coordination::query_runs_filtered(None, None, None, Some(200), None) {
            Ok(fresh) => {
                let fresh_map: HashMap<String, RunSummaryRow> = fresh
                    .iter()
                    .cloned()
                    .map(|r| (r.run_id.clone(), r))
                    .collect();
                for id in &self.selected_run_ids {
                    if !fresh_map.contains_key(id.as_str()) {
                        warn!(run_id = %id, "selected run no longer in fresh data — will be removed from list");
                    }
                }
                if let Some(id) = &self.selected_run_id {
                    if !fresh_map.contains_key(id.as_str()) {
                        warn!(run_id = %id, "primary selected run no longer in fresh data");
                    }
                }
                self.runs = fresh;
                self.run_sidebar_prefixes = self
                    .runs
                    .iter()
                    .filter_map(|run| {
                        coordination::load_run(&run.run_id).ok().map(|loaded| {
                            (
                                run.run_id.clone(),
                                format!(
                                    "[Workflow: {}] [Workspace: {}] [Host: {}]",
                                    loaded.workflow_mode.as_str(),
                                    loaded.workspace_path,
                                    loaded.execution_environment.as_str()
                                ),
                            )
                        })
                    })
                    .collect();
            }
            Err(err) => self.append_error(format!("failed to query runs: {err}")),
        }
    }}
impl AutoRunsApp {
    fn select_run(&mut self, run_id: String, is_shift_click: bool) {
        if is_shift_click {
            if self.selection_anchor_run_id.is_none() {
                self.selection_anchor_run_id = Some(run_id.clone());
            }
            if let Some(anchor) = &self.selection_anchor_run_id.clone() {
                let anchor_idx = self.runs.iter().position(|r| &r.run_id == anchor);
                let clicked_idx = self.runs.iter().position(|r| r.run_id == run_id);
                if let (Some(a), Some(c)) = (anchor_idx, clicked_idx) {
                    let (start, end) = if a < c { (a, c) } else { (c, a) };
                    for run in &self.runs[start..=end] {
                        self.selected_run_ids.insert(run.run_id.clone());
                    }
                }
                self.selected_run_ids.insert(run_id.clone());
                self.selection_anchor_run_id = Some(run_id.clone());
                self.selected_run_id = Some(run_id);
            } else {
                self.selected_run_ids.insert(run_id.clone());
                self.selection_anchor_run_id = Some(run_id.clone());
                self.selected_run_id = Some(run_id);
            }
        } else {
            self.selected_run_ids.clear();
            self.selected_run_ids.insert(run_id.clone());
            self.selection_anchor_run_id = Some(run_id.clone());
            self.selected_run_id = Some(run_id);
        }

        self.populate_form_on_next_snapshot = true;
        self.snapshot = None;
        self.events.clear();
        self.selected_run_load_error = None;
        self.request_selected_run_load();
    }}
impl AutoRunsApp {
    fn request_selected_run_load(&mut self) {
        let Some(run_id) = self.selected_run_id.clone() else {
            return;
        };
        if self.selected_run_load_in_flight.as_deref() == Some(run_id.as_str()) {
            return;
        }
        if self.selected_run_load_in_flight.is_some() {
            self.selected_run_load_pending = true;
            return;
        }
        let Some(tx) = &self.cmd_tx else {
            return;
        };
        let tx = tx.clone();
        debug!("UI thread: sending LoadSelectedRun for {run_id}");
        if let Err(err) = tx.try_send(UiThreadCmd::LoadSelectedRun(run_id)) {
            self.selected_run_load_pending = true;
            debug!(error = %err, "UI thread: failed to queue LoadSelectedRun command");
            return;
        }
        self.selected_run_load_in_flight = self.selected_run_id.clone();
    }}
impl AutoRunsApp {
    fn request_runs_refresh(&mut self) {
        if self.refresh_runs_in_flight {
            return;
        }
        let Some(tx) = &self.cmd_tx else {
            return;
        };
        let tx = tx.clone();
        debug!("UI thread: sending RefreshRuns command");
        if let Err(err) = tx.try_send(UiThreadCmd::RefreshRuns) {
            debug!(error = %err, "UI thread: failed to queue RefreshRuns command");
            return;
        }
        self.refresh_runs_in_flight = true;
    }}
impl AutoRunsApp {
    fn selected_run_summary(&self) -> Option<&RunSummaryRow> {
        let selected_run_id = self.selected_run_id.as_deref()?;
        self.runs.iter().find(|run| run.run_id == selected_run_id)
    }}
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
fn run_completion_criteria_input(app: &AutoRunsApp) -> Option<String> {
    let criteria = app.completion_criteria.trim();
    if criteria.is_empty() {
        return None;
    }
    Some(criteria.to_string())
}
fn main() {}
