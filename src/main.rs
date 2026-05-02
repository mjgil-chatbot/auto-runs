use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::Stdio;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};
use tokio::sync::mpsc as tokio_mpsc;

mod restart_dialog;
mod run_form;

use auto_runs::coordination::{
    self, CoordinationEvent, CoordinationRunSnapshot, CoordinationTaskStatus, RunSummaryRow,
    UpsertRunInput,
};
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

// ─── Async UI thread infrastructure ───────────────────────────────────────────

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

/// Runs the background thread that handles all blocking coordination calls.
/// The UI sends commands via `cmd_tx` and receives results via `result_rx`.
fn spawn_ui_background_thread() -> (
    tokio::sync::mpsc::Sender<UiThreadCmd>,
    Receiver<UiThreadResult>,
) {
    let (cmd_tx, mut cmd_rx) = tokio_mpsc::channel::<UiThreadCmd>(32);
    let (result_tx, result_rx) = channel::<UiThreadResult>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let mut __tick = 0;
            while let Some(cmd) = cmd_rx.recv().await {
                __tick += 1;
                eprintln!("BG: tick {} got cmd {:?}", __tick, cmd);
                let result_tx = result_tx.clone();
                match cmd {
                    UiThreadCmd::RefreshRuns => {
                        let result =
                            coordination::query_runs_filtered(None, None, None, Some(200), None)
                                .map_err(|e| e.to_string());
                        let runs = match result {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("RefreshRuns failed: {e}");
                                Vec::new()
                            }
                        };
                        let _ = result_tx.send(UiThreadResult::Runs(runs));
                    }
                    UiThreadCmd::LoadSelectedRun(run_id) => {
                        let snapshot =
                            coordination::load_run_snapshot(&run_id).map_err(|e| e.to_string());
                        let events = coordination::load_events(&run_id, Some(40), None)
                            .map_err(|e| e.to_string());
                        let _ = result_tx.send(UiThreadResult::LoadSelectedRunResult {
                            run_id,
                            snapshot,
                            events,
                        });
                    }
                    UiThreadCmd::StopRun(run_id) => {
                        let snapshot =
                            coordination::stop_run(&run_id, "Stopped from auto-runs UI.")
                                .map_err(|e| e.to_string());
                        let _ = result_tx.send(UiThreadResult::StopRunResult { run_id, snapshot });
                    }
                    UiThreadCmd::RestartFailedTasks(run_id) => {
                        let snapshot =
                            coordination::restart_failed_tasks(&run_id).map_err(|e| e.to_string());
                        let _ = result_tx.send(UiThreadResult::RestartResult { run_id, snapshot });
                    }
                }
            }
        });
    });

    (cmd_tx, result_rx)
}

fn main() -> anyhow::Result<()> {
    auto_runs::process_limits::apply_default_process_memory_limit("auto-runs");
    let _log_guard = auto_runs::debug::init();
    let _debug_guard = rust_debug::init_debug_defaults("auto-runs");
    rust_debug::install_panic_hook();
    auto_runs::ensure_path_symlink();
    info!("auto-runs application start");

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        match args[1].as_str() {
            "drive-forever" => {
                driver::run_background_driver_loop()?;
                return Ok(());
            }
            other => anyhow::bail!("unknown auto-runs command `{other}`"),
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("auto-runs")
            .with_inner_size([1330.0, 890.0])
            .with_min_inner_size([960.0, 640.0]),
        ..Default::default()
    };

    let (cmd_tx, result_rx) = spawn_ui_background_thread();

    eframe::run_native(
        "Auto Runs",
        options,
        Box::new(move |_cc| {
            let app = AutoRunsApp {
                cmd_tx: Some(cmd_tx),
                result_rx: Some(result_rx),
                ..Default::default()
            };
            Ok(Box::new(app))
        }),
    )
    .map_err(|err| anyhow::anyhow!("failed to launch auto-runs UI: {err}"))?;
    Ok(())
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
    }

    fn append_error(&mut self, error: impl Into<String>) {
        let message = error.into();
        warn!(error = %message, "auto-runs ui action failed");
        self.errors.push(message);
        if self.errors.len() > 8 {
            self.errors.remove(0);
        }
    }

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
    }

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
    }

    fn set_execution_environment(&mut self, environment: AutonomousExecutionEnvironment) {
        self.execution_environment = environment;
        if self.execution_environment == AutonomousExecutionEnvironment::Runsc
            && self.runsc_workspace_includes.trim().is_empty()
        {
            self.runsc_workspace_includes = ".\n".to_string();
        }
    }

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
    }

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
    }

    fn load_selected_run(&mut self) {
        let Some(run_id) = self.selected_run_id.clone() else {
            self.snapshot = None;
            self.events.clear();
            return;
        };

        match coordination::load_run_snapshot(&run_id) {
            Ok(snapshot) => self.snapshot = Some(snapshot),
            Err(err) => {
                self.snapshot = None;
                self.append_error(format!("failed to load run snapshot {run_id}: {err}"));
            }
        }

        match coordination::load_events(&run_id, Some(40), None) {
            Ok(events) => self.events = events,
            Err(err) => {
                self.events.clear();
                self.append_error(format!("failed to load events {run_id}: {err}"));
            }
        }
    }

    fn create_run(&mut self) {
        let root_chat_id = format!("auto-runs-{}", Utc::now().timestamp_millis());
        let mut notes = self.notes.trim().to_string();
        if self.workflow_mode == AutonomousWorkflowMode::PerfHillClimber {
            notes = notes_with_setting(
                self.notes.trim(),
                "bench_script_path",
                self.bench_script_path.trim(),
            );
        }
        let lightweight_worker_providers =
            run_form::selected_lightweight_worker_models(&self.lightweight_worker_models);
        if self.workflow_mode == AutonomousWorkflowMode::LightweightImplementation {
            notes = coordination::lightweight_notes_with_settings(
                &notes,
                self.lightweight_task_doc_path.trim(),
                self.lightweight_task_splitter_provider.trim(),
                &lightweight_worker_providers,
            );
            notes = coordination::lightweight_notes_with_validation_provider(
                &notes,
                self.lightweight_validation_model.trim(),
            );
            notes = coordination::lightweight_notes_with_repo_configuration(
                &notes,
                self.lightweight_repo_configuration,
            );
        }
        if self.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
            notes = coordination::github_branch_fix_notes_with_settings(
                &notes,
                self.github_branch_fix_branch.trim(),
                self.github_branch_fix_poll_minutes,
            );
        }
        let includes = self
            .runsc_workspace_includes
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        let review_scope = match &self.review_scope {
            ReviewScope::Diff { .. } => Some(ReviewScope::Diff {
                base: self.review_diff_base.trim().to_string(),
            }),
            ReviewScope::Paths(_) => Some(ReviewScope::Paths(
                self.review_paths
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
            )),
            ReviewScope::Globs(_) => Some(ReviewScope::Globs(
                self.review_paths
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_string)
                    .collect(),
            )),
            ReviewScope::Worktree => Some(ReviewScope::Worktree),
        };
        match coordination::upsert_run(UpsertRunInput {
            existing_run_id: None,
            root_chat_id,
            workspace_path: self.workspace_path.trim().to_string(),
            execution_environment: self.execution_environment.clone(),
            runsc_workspace_includes: includes,
            provider: if self.workflow_mode == AutonomousWorkflowMode::LightweightImplementation {
                self.lightweight_task_splitter_provider.trim().to_string()
            } else {
                self.provider.trim().to_string()
            },
            goal: self.goal.trim().to_string(),
            workflow_mode: self.workflow_mode.clone(),
            design_doc_path: self.design_doc_path.trim().to_string(),
            completion_criteria: run_form::run_completion_criteria_input(self),
            max_iterations: self.max_iterations.max(1),
            created_by_instance: None,
            max_agents: if matches!(
                self.workflow_mode,
                AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
            ) {
                1
            } else {
                self.max_agents.max(1)
            },
            allow_write_tasks: self.allow_write_tasks,
            require_review_before_apply: self.require_review_before_apply,
            auto_retry_failed_tasks: self.auto_retry_failed_tasks,
            notes,
            review_scope: (self.workflow_mode == AutonomousWorkflowMode::CodeReview)
                .then_some(review_scope.unwrap_or(ReviewScope::Worktree)),
            review_dimensions: (self.workflow_mode == AutonomousWorkflowMode::CodeReview)
                .then_some(Vec::new()),
            review_fix_mode: (self.workflow_mode == AutonomousWorkflowMode::CodeReview)
                .then_some(self.review_fix_mode.clone()),
            council_turn_policy: (self.workflow_mode == AutonomousWorkflowMode::FreeformCouncil)
                .then_some(self.council_turn_policy.clone()),
            council_max_turns: (self.workflow_mode == AutonomousWorkflowMode::FreeformCouncil)
                .then_some(self.council_max_turns),
        }) {
            Ok(snapshot) => {
                let new_run_id = snapshot.run.run_id.clone();
                self.selected_run_id = Some(new_run_id.clone());
                self.selected_run_ids.clear();
                self.selected_run_ids.insert(new_run_id.clone());
                self.selection_anchor_run_id = Some(new_run_id.clone());
                self.populate_form_on_next_snapshot = false;
                self.snapshot = Some(snapshot);
                self.refresh_runs();
                self.load_selected_run();
                run_form::populate_form_from_snapshot(self);
            }
            Err(err) => self.append_error(format!("failed to create run: {err}")),
        }
    }

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
    }

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
    }

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
    }

    fn selected_run_summary(&self) -> Option<&RunSummaryRow> {
        let selected_run_id = self.selected_run_id.as_deref()?;
        self.runs.iter().find(|run| run.run_id == selected_run_id)
    }

    fn render_snapshot_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Run Detail");
            if let Some(run_id) = self.selected_run_id.clone() {
                if ui.button("Restart Failed").clicked() {
                    self.open_restart_dialog();
                }
                if ui.button("Stop").clicked() {
                    if let Some(tx) = &self.cmd_tx {
                        let run_id = run_id.clone();
                        let tx = tx.clone();
                        if let Err(err) = tx.try_send(UiThreadCmd::StopRun(run_id)) {
                            debug!(error = %err, "UI thread: failed to queue StopRun command");
                        }
                    }
                }
            }
        });

        if let Some(run_provider) = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.run.provider.clone())
        {
            self.render_restart_dialog(ui, &run_provider);
        }

        let Some(snapshot) = self.snapshot.as_ref() else {
            if let Some(run) = self.selected_run_summary() {
                ui.label(format!("Run ID: {}", run.run_id));
                ui.label(format!("Provider: {}", run.provider));
                ui.label(format!("Status: {}", run.status));
                if let Some(workflow) = &run.workflow_mode {
                    ui.label(format!("Workflow: {}", workflow));
                }
                ui.separator();
                if let Some(error) = &self.selected_run_load_error {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!("Failed to load run details: {error}"),
                    );
                } else {
                    ui.label("Loading run details...");
                }
            } else {
                ui.label("Select a run.");
            }
            return;
        };

        ui.label(format!("Run ID: {}", snapshot.run.run_id));
        ui.label(format!("Provider: {}", snapshot.run.provider));
        ui.label(format!("Status: {}", snapshot.run.status.as_str()));
        ui.label(format!(
            "Workflow: {}",
            snapshot.run.workflow_mode.display_name()
        ));
        ui.label(format!(
            "Execution Environment: {}",
            snapshot.run.execution_environment.as_str()
        ));
        ui.label(format!("Workspace: {}", snapshot.run.workspace_path));
        run_form::render_run_settings_summary(&snapshot.run, ui);
        if snapshot.run.workflow_mode == AutonomousWorkflowMode::PerfHillClimber {
            if let Some(path) = note_setting_value(&snapshot.run.notes, "bench_script_path") {
                ui.label(format!("Bench Script: {}", path));
            }
        }
        if snapshot.run.execution_environment == AutonomousExecutionEnvironment::Runsc
            && !snapshot.run.runsc_workspace_includes.is_empty()
        {
            ui.label(format!(
                "Runsc Includes: {}",
                snapshot.run.runsc_workspace_includes.join(", ")
            ));
        }

        let mut counts = BTreeMap::new();
        for task in &snapshot.tasks {
            *counts.entry(format!("{:?}", task.status)).or_insert(0usize) += 1;
        }
        ui.separator();
        ui.label("Task Status");
        for status in [
            CoordinationTaskStatus::Queued,
            CoordinationTaskStatus::Claiming,
            CoordinationTaskStatus::Running,
            CoordinationTaskStatus::Completed,
            CoordinationTaskStatus::Failed,
            CoordinationTaskStatus::Blocked,
            CoordinationTaskStatus::Canceled,
        ] {
            let count = counts
                .get(&format!("{:?}", status))
                .copied()
                .unwrap_or_default();
            ui.label(format!("{:?}: {}", status, count));
        }
        ui.separator();
        ui.label(format!("Total: {} tasks", snapshot.tasks.len()));
        ui.separator();

        let selected_run_id = snapshot.run.run_id.clone();
        let tasks_snapshot: Vec<_> = snapshot.tasks.clone();
        egui::ScrollArea::vertical()
            .max_height(500.0)
            .show(ui, |ui| {
                for task in &tasks_snapshot {
                    ui.horizontal(|ui| {
                        ui.label(format!("{:?}", task.status));
                        let label = if task.title.len() > 60 {
                            format!("{}...", &task.title[..60])
                        } else {
                            task.title.clone()
                        };
                        ui.label(label);
                        if matches!(
                            task.status,
                            CoordinationTaskStatus::Failed | CoordinationTaskStatus::Canceled
                        ) {
                            if ui.add(egui::Button::new("Restart").small()).clicked() {
                                self.selected_run_id = Some(selected_run_id.clone());
                                self.restart_dialog.selected_task_id = task.task_id.clone();
                                self.open_restart_dialog();
                            }
                        }
                    });
                }
            });

        ui.separator();
        ui.heading("Recent Events");
        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for event in self.events.iter().rev().take(40) {
                    ui.label(run_form::format_recent_event_line(event));
                }
            });
    }

    fn render_create_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("New Run").clicked() {
                self.start_new_run();
            }
            ui.heading("Run Draft");
        });
        ui.label("Goal");
        ui.text_edit_multiline(&mut self.goal);
        ui.label("Workflow");
        egui::ComboBox::from_id_source("workflow")
            .selected_text(self.workflow_mode.display_name())
            .show_ui(ui, |ui| {
                for mode in AutonomousWorkflowMode::all_variants() {
                    let selected = self.workflow_mode == *mode;
                    if ui.selectable_label(selected, mode.display_name()).clicked() {
                        self.set_workflow_mode(mode.clone());
                    }
                }
            });
        ui.horizontal(|ui| {
            if ui.button("[auto]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::DesignConsensus);
            }
            if ui.button("[lightweight]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::LightweightImplementation);
            }
            if ui.button("[branch-fix]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::GithubBranchFix);
            }
            if ui.button("[ralph]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::RalphLoop);
            }
            if ui.button("[review]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::CodeReview);
            }
            if ui.button("[council]").clicked() {
                self.set_workflow_mode(AutonomousWorkflowMode::FreeformCouncil);
            }
        });
        ui.label("Workspace");
        ui.text_edit_singleline(&mut self.workspace_path);
        if self.workflow_mode == AutonomousWorkflowMode::PerfHillClimber {
            ui.label("Bench Script");
            ui.label("Absolute path to the benchmark shell script for this perf run.");
            ui.text_edit_singleline(&mut self.bench_script_path);
        }
        run_form::render_workflow_settings(self, ui);
        ui.label("Execution Environment");
        egui::ComboBox::from_id_source("execution_environment")
            .selected_text(self.execution_environment.as_str())
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(
                        self.execution_environment == AutonomousExecutionEnvironment::Host,
                        AutonomousExecutionEnvironment::Host.as_str(),
                    )
                    .clicked()
                {
                    self.set_execution_environment(AutonomousExecutionEnvironment::Host);
                }
                if ui
                    .selectable_label(
                        self.execution_environment == AutonomousExecutionEnvironment::Runsc,
                        AutonomousExecutionEnvironment::Runsc.as_str(),
                    )
                    .clicked()
                {
                    self.set_execution_environment(AutonomousExecutionEnvironment::Runsc);
                }
            });
        if self.execution_environment == AutonomousExecutionEnvironment::Runsc {
            ui.label("Runsc Workspace Includes");
            ui.label("One workspace path per line. Use `.` to mount the full workspace.");
            ui.text_edit_multiline(&mut self.runsc_workspace_includes);
        }
        if self.workflow_mode == AutonomousWorkflowMode::RalphLoop {
            ui.label("Provider");
            egui::ComboBox::from_id_source("provider")
                .selected_text(self.provider.as_str())
                .show_ui(ui, |ui| {
                    for provider in ModelProvider::ALL {
                        ui.selectable_value(
                            &mut self.provider,
                            provider.as_str().to_string(),
                            provider.display_name(),
                        );
                    }
                });
        }
        if self.workflow_mode == AutonomousWorkflowMode::RalphLoop {
            ui.horizontal(|ui| {
                ui.label("Max Iterations");
                ui.add(egui::DragValue::new(&mut self.max_iterations).range(1..=100));
            });
        } else if !matches!(
            self.workflow_mode,
            AutonomousWorkflowMode::GithubBranchFix
                | AutonomousWorkflowMode::LightweightImplementation
        ) {
            ui.horizontal(|ui| {
                ui.label("Max Agents");
                ui.add(egui::DragValue::new(&mut self.max_agents).range(1..=32));
            });
        }
        ui.checkbox(&mut self.allow_write_tasks, "Allow write tasks");
        ui.checkbox(
            &mut self.require_review_before_apply,
            "Require review before apply",
        );
        ui.checkbox(&mut self.auto_retry_failed_tasks, "Auto-retry failed tasks");
        ui.label("Design Doc");
        if self.workflow_mode == AutonomousWorkflowMode::LightweightImplementation {
            ui.label("Optional bootstrap target. The splitter creates the initial design doc here if it does not already exist.");
        } else if self.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
            ui.label("Optional bootstrap target. The branch-fix worker creates the initial design doc here if it does not already exist.");
        }
        ui.text_edit_singleline(&mut self.design_doc_path);
        run_form::render_completion_criteria(self, ui);
        ui.label("Notes");
        ui.text_edit_multiline(&mut self.notes);
        if ui.button("Create Run").clicked() {
            self.create_run();
        }
    }

    fn render_runs_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Runs");
        ui.horizontal(|ui| {
            if ui.button("Refresh").clicked() {
                if let Some(tx) = &self.cmd_tx {
                    let tx = tx.clone();
                    let _ = tx.try_send(UiThreadCmd::RefreshRuns);
                }
            }
        });
        let mut pending_select: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for run in &self.runs {
                    let prefix = self
                        .run_sidebar_prefixes
                        .get(&run.run_id)
                        .map(String::as_str);
                    let selected = self.selected_run_ids.contains(&run.run_id);
                    if run_form::render_run_sidebar_entry(ui, run, prefix, selected).clicked() {
                        pending_select = Some(run.run_id.clone());
                    }
                }
            });
        if let Some(run_id) = pending_select {
            self.select_run(run_id, false);
        }
    }
}

impl eframe::App for AutoRunsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ensure_defaults();
        self.ensure_background_driver();

        // Drain results from the background thread.
        let mut new_runs: Option<Vec<RunSummaryRow>> = None;
        let mut new_snapshot: Option<Option<CoordinationRunSnapshot>> = None;
        let mut new_events: Option<Result<Vec<CoordinationEvent>, String>> = None;
        if let Some(rx) = &self.result_rx {
            while let Ok(result) = rx.try_recv() {
                match result {
                    UiThreadResult::Runs(runs) => {
                        self.refresh_runs_in_flight = false;
                        new_runs = Some(runs);
                    }
                    UiThreadResult::LoadSelectedRunResult {
                        run_id,
                        snapshot,
                        events,
                    } => {
                        if self.selected_run_load_in_flight.as_deref() == Some(run_id.as_str()) {
                            self.selected_run_load_in_flight = None;
                        }
                        if self.selected_run_id.as_deref() == Some(run_id.as_str()) {
                            match snapshot {
                                Ok(snapshot) => {
                                    self.selected_run_load_error = None;
                                    new_snapshot = Some(Some(snapshot));
                                }
                                Err(err) => {
                                    self.selected_run_load_error = Some(err);
                                    new_snapshot = Some(None);
                                }
                            }
                            new_events = Some(events);
                        } else {
                            self.selected_run_load_pending = true;
                        }
                    }
                    UiThreadResult::StopRunResult { snapshot, .. }
                    | UiThreadResult::RestartResult { snapshot, .. } => {
                        new_snapshot = Some(snapshot.ok());
                    }
                }
            }
        }

        if let Some(runs) = new_runs {
            self.runs = runs;
            let run_sidebar_prefixes: HashMap<String, String> = self
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
            self.run_sidebar_prefixes = run_sidebar_prefixes;
        }

        if let Some(snapshot) = new_snapshot {
            self.snapshot = snapshot;
            if self.snapshot.is_some() && self.populate_form_on_next_snapshot {
                run_form::populate_form_from_snapshot(self);
            }
            self.populate_form_on_next_snapshot = false;
        }

        if let Some(events_result) = new_events {
            match events_result {
                Ok(ev) => self.events = ev,
                Err(e) => {
                    self.events.clear();
                    if let Some(run_id) = self.selected_run_id.clone() {
                        self.append_error(format!("failed to load events {run_id}: {e}"));
                    }
                }
            }
        }

        let should_poll = self
            .last_poll
            .is_none_or(|last_poll| last_poll.elapsed() >= POLL_INTERVAL);
        if should_poll {
            // Send refresh command to background thread — no blocking on UI.
            self.request_runs_refresh();
            if self.selected_run_id.is_some() {
                self.request_selected_run_load();
            }
            self.last_poll = Some(Instant::now());
        }

        if self.selected_run_load_pending && self.selected_run_load_in_flight.is_none() {
            self.selected_run_load_pending = false;
            self.request_selected_run_load();
        }

        egui::TopBottomPanel::bottom("errors")
            .resizable(true)
            .show(ctx, |ui| {
                if !self.errors.is_empty() {
                    ui.heading("Errors");
                    for error in self.errors.iter().rev() {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                }
            });

        egui::SidePanel::left("create")
            .resizable(true)
            .default_width(360.0)
            .show(ctx, |ui| self.render_create_panel(ui));
        egui::SidePanel::right("runs")
            .resizable(true)
            .default_width(340.0)
            .show(ctx, |ui| self.render_runs_panel(ui));
        egui::CentralPanel::default().show(ctx, |ui| self.render_snapshot_panel(ui));

        ctx.request_repaint_after(POLL_INTERVAL);
    }
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

fn run_completion_criteria_input(app: &AutoRunsApp) -> Option<String> {
    let criteria = app.completion_criteria.trim();
    if criteria.is_empty() {
        return None;
    }
    Some(criteria.to_string())
}
