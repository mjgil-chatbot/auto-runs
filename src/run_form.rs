use super::{
    coordination, note_setting_value, AutoRunsApp, CoordinationEvent, ReviewScope, RunSummaryRow,
};
use auto_runs::model::ModelProvider;
use eframe::egui;

pub fn populate_form_from_snapshot(app: &mut AutoRunsApp) {
    let Some(run) = app.snapshot.as_ref().map(|snapshot| snapshot.run.clone()) else {
        return;
    };

    app.goal = run.goal.clone();
    app.workspace_path = run.workspace_path.clone();
    app.bench_script_path = note_setting_value(&run.notes, "bench_script_path")
        .unwrap_or_default()
        .to_string();
    app.design_doc_path = run.design_doc_path.clone();
    app.lightweight_task_doc_path = note_setting_value(&run.notes, "lightweight_task_doc")
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| coordination::lightweight_default_task_doc_path(&run.design_doc_path));
    app.lightweight_task_splitter_provider =
        note_setting_value(&run.notes, "lightweight_task_splitter")
            .unwrap_or_default()
            .to_string();
    app.lightweight_validation_model =
        note_setting_value(&run.notes, coordination::LIGHTWEIGHT_VALIDATION_NOTE_KEY)
            .unwrap_or(app.lightweight_task_splitter_provider.as_str())
            .to_string();
    app.lightweight_worker_models = parse_lightweight_worker_models(note_setting_value(
        &run.notes,
        "lightweight_worker_models",
    ));
    if app.lightweight_worker_models.is_empty() {
        let default_worker = if app.lightweight_task_splitter_provider.trim().is_empty() {
            ModelProvider::Codex.as_str().to_string()
        } else {
            app.lightweight_task_splitter_provider.clone()
        };
        app.lightweight_worker_models = vec![default_worker];
    }
    app.lightweight_repo_configuration = coordination::lightweight_repo_configuration_for_run(&run);
    app.github_branch_fix_branch =
        note_setting_value(&run.notes, coordination::GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY)
            .unwrap_or_default()
            .to_string();
    app.github_branch_fix_poll_minutes = note_setting_value(
        &run.notes,
        coordination::GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY,
    )
    .and_then(|value| value.parse::<u32>().ok())
    .filter(|value| *value > 0)
    .unwrap_or(5);
    app.completion_criteria = run.completion_criteria.clone();
    app.notes = notes_without_settings(
        &run.notes,
        &[
            "bench_script_path",
            "lightweight_task_doc",
            "lightweight_task_doc_cycle",
            "lightweight_task_splitter",
            "lightweight_worker_models",
            "lightweight_validation_model",
            coordination::LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY,
            coordination::GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY,
            coordination::GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY,
            "perf_worktree_root",
            "perf_worktree_path",
        ],
    );
    app.provider = run.provider.clone();
    app.workflow_mode = run.workflow_mode.clone();
    app.execution_environment = run.execution_environment.clone();
    app.runsc_workspace_includes = run.runsc_workspace_includes.join("\n");
    app.max_iterations = run.max_iterations.max(1);
    app.max_agents = run.max_agents.max(1);
    app.allow_write_tasks = run.allow_write_tasks;
    app.require_review_before_apply = run.require_review_before_apply;
    app.auto_retry_failed_tasks = run.auto_retry_failed_tasks;
    app.review_fix_mode = run.review_fix_mode.clone().unwrap_or_default();
    app.council_turn_policy = run.council_turn_policy.clone().unwrap_or_default();
    app.council_max_turns = run.council_max_turns.unwrap_or(40).max(1);

    match run.review_scope.clone().unwrap_or_default() {
        ReviewScope::Worktree => {
            app.review_scope = ReviewScope::Worktree;
            app.review_diff_base.clear();
            app.review_paths.clear();
        }
        ReviewScope::Diff { base } => {
            app.review_scope = ReviewScope::Diff { base: base.clone() };
            app.review_diff_base = base;
            app.review_paths.clear();
        }
        ReviewScope::Paths(paths) => {
            app.review_scope = ReviewScope::Paths(paths.clone());
            app.review_paths = paths.join("\n");
            app.review_diff_base.clear();
        }
        ReviewScope::Globs(globs) => {
            app.review_scope = ReviewScope::Globs(globs.clone());
            app.review_paths = globs.join("\n");
            app.review_diff_base.clear();
        }
    }
}

pub fn recent_events_text(events: &[CoordinationEvent]) -> String {
    events
        .iter()
        .rev()
        .take(40)
        .map(format_recent_event_line)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn format_recent_event_line(event: &CoordinationEvent) -> String {
    let provider = event.provider.as_deref().unwrap_or("system");
    format!(
        "{} [{}] {}",
        event.at.format("%H:%M:%S"),
        provider,
        event.message
    )
}

fn notes_without_settings(notes: &str, keys: &[&str]) -> String {
    notes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !keys.iter().any(|key| line.starts_with(&format!("{key}="))))
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn selected_lightweight_worker_models(worker_models: &[String]) -> Vec<String> {
    let selected = worker_models
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        vec![ModelProvider::Codex.as_str().to_string()]
    } else {
        selected
    }
}

pub fn run_completion_criteria_input(app: &AutoRunsApp) -> String {
    if app.workflow_mode == auto_runs::AutonomousWorkflowMode::GithubBranchFix {
        String::new()
    } else {
        app.completion_criteria.trim().to_string()
    }
}

pub fn render_completion_criteria(app: &mut AutoRunsApp, ui: &mut egui::Ui) {
    if app.workflow_mode == auto_runs::AutonomousWorkflowMode::GithubBranchFix {
        return;
    }
    ui.label("Completion Criteria");
    ui.text_edit_multiline(&mut app.completion_criteria);
}

pub fn run_sidebar_label(
    run: &RunSummaryRow,
    prefix: Option<&str>,
    style: &egui::Style,
    visuals: &egui::Visuals,
) -> egui::WidgetText {
    let mut job = egui::text::LayoutJob::default();
    append_run_sidebar_segment(
        &mut job,
        &format!("[{}] ", run.status),
        run_sidebar_status_color(&run.status, visuals),
        style,
    );
    match prefix.and_then(parse_run_sidebar_prefix) {
        Some((workflow, workspace, host)) => {
            let workspace = sidebar_workspace_display_text(workspace);
            append_run_sidebar_segment(
                &mut job,
                &format!("[{workflow}] "),
                egui::Color32::from_rgb(140, 210, 140),
                style,
            );
            append_run_sidebar_segment(
                &mut job,
                &format!("[{workspace}] "),
                egui::Color32::from_rgb(145, 195, 245),
                style,
            );
            append_run_sidebar_segment(
                &mut job,
                &format!("[{host}] "),
                egui::Color32::from_rgb(220, 220, 220),
                style,
            );
        }
        None => {
            if let Some(prefix) = prefix {
                append_run_sidebar_segment(
                    &mut job,
                    &format!("{prefix} "),
                    visuals.text_color(),
                    style,
                );
            }
        }
    }
    append_run_sidebar_segment(
        &mut job,
        &format!("{} ", run.goal),
        visuals.text_color(),
        style,
    );
    job.into()
}

pub fn render_run_sidebar_entry(
    ui: &mut egui::Ui,
    run: &RunSummaryRow,
    prefix: Option<&str>,
    selected: bool,
) -> egui::Response {
    let label = run_sidebar_label(run, prefix, ui.style().as_ref(), ui.visuals());
    ui.add_sized(
        [ui.available_width(), 0.0],
        egui::Button::new(label)
            .selected(selected)
            .frame(false)
            .truncate(),
    )
}

fn parse_run_sidebar_prefix(prefix: &str) -> Option<(&str, &str, &str)> {
    let prefix = prefix.strip_prefix("[Workflow: ")?;
    let (workflow, rest) = prefix.split_once("] [Workspace: ")?;
    let (workspace, host) = rest.split_once("] [Host: ")?;
    Some((workflow, workspace, host.strip_suffix(']')?))
}

fn sidebar_workspace_display_text(workspace: &str) -> &str {
    // Strip the user's home directory prefix for cleaner display
    if let Some(home) = dirs::home_dir() {
        if let Some(home_str) = home.to_str() {
            if workspace.starts_with(home_str) {
                return workspace.strip_prefix(home_str).unwrap_or(workspace);
            }
        }
    }
    workspace
}

fn append_run_sidebar_segment(
    job: &mut egui::text::LayoutJob,
    text: &str,
    color: egui::Color32,
    style: &egui::Style,
) {
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: egui::TextStyle::Button.resolve(style),
            color,
            ..Default::default()
        },
    );
}

fn run_sidebar_status_color(status: &str, visuals: &egui::Visuals) -> egui::Color32 {
    match status {
        "failed" => egui::Color32::from_rgb(185, 45, 45),
        "completed" => egui::Color32::from_rgb(35, 120, 35),
        _ => visuals.text_color(),
    }
}

pub fn render_lightweight_settings(app: &mut AutoRunsApp, ui: &mut egui::Ui) {
    ui.label("Task List Doc");
    ui.label("Optional bootstrap target. The splitter creates it if missing and review passes advance it.");
    ui.text_edit_singleline(&mut app.lightweight_task_doc_path);
    ui.label("Task Splitting Model");
    egui::ComboBox::from_id_salt("lightweight_task_splitter_provider")
        .selected_text(app.lightweight_task_splitter_provider.as_str())
        .show_ui(ui, |ui| {
            for provider in ModelProvider::ALL {
                ui.selectable_value(
                    &mut app.lightweight_task_splitter_provider,
                    provider.as_str().to_string(),
                    provider.display_name(),
                );
            }
        });
    ui.label("Validation Model");
    egui::ComboBox::from_id_salt("lightweight_validation_model")
        .selected_text(app.lightweight_validation_model.as_str())
        .show_ui(ui, |ui| {
            for provider in ModelProvider::ALL {
                ui.selectable_value(
                    &mut app.lightweight_validation_model,
                    provider.as_str().to_string(),
                    provider.display_name(),
                );
            }
        });
    ui.label("Worker Models");
    ui.label("Add one row per implementation worker model.");
    render_lightweight_worker_model_rows(ui, &mut app.lightweight_worker_models);
    ui.horizontal(|ui| {
        ui.label("Max Agents");
        ui.add(egui::DragValue::new(&mut app.max_agents).range(1..=32));
    });
    ui.label("Repo Configuration");
    ui.label("Optional repo bootstrap configuration for create-repo runs.");
    egui::ComboBox::from_id_salt("lightweight_repo_configuration")
        .selected_text(app.lightweight_repo_configuration.display_name())
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut app.lightweight_repo_configuration,
                coordination::LightweightRepoConfiguration::None,
                coordination::LightweightRepoConfiguration::None.display_name(),
            );
            ui.selectable_value(
                &mut app.lightweight_repo_configuration,
                coordination::LightweightRepoConfiguration::Rust,
                coordination::LightweightRepoConfiguration::Rust.display_name(),
            );
            ui.selectable_value(
                &mut app.lightweight_repo_configuration,
                coordination::LightweightRepoConfiguration::Zig,
                coordination::LightweightRepoConfiguration::Zig.display_name(),
            );
        });
}

pub fn render_github_branch_fix_settings(app: &mut AutoRunsApp, ui: &mut egui::Ui) {
    ui.label("Worker Model");
    egui::ComboBox::from_id_salt("github_branch_fix_worker_model")
        .selected_text(app.provider.as_str())
        .show_ui(ui, |ui| {
            for provider in ModelProvider::ALL {
                ui.selectable_value(
                    &mut app.provider,
                    provider.as_str().to_string(),
                    provider.display_name(),
                );
            }
        });
    ui.label("Target Branch");
    ui.label("Poll and repair GitHub Actions runs for this branch.");
    ui.text_edit_singleline(&mut app.github_branch_fix_branch);
    ui.horizontal(|ui| {
        ui.label("Poll Minutes");
        ui.add(egui::DragValue::new(&mut app.github_branch_fix_poll_minutes).range(1..=1440));
    });
}

pub fn render_workflow_settings(app: &mut AutoRunsApp, ui: &mut egui::Ui) {
    if app.workflow_mode == auto_runs::AutonomousWorkflowMode::LightweightImplementation {
        render_lightweight_settings(app, ui);
    } else if app.workflow_mode == auto_runs::AutonomousWorkflowMode::GithubBranchFix {
        render_github_branch_fix_settings(app, ui);
    }
}

pub fn render_run_settings_summary(run: &coordination::CoordinationRun, ui: &mut egui::Ui) {
    if run.workflow_mode == auto_runs::AutonomousWorkflowMode::GithubBranchFix {
        ui.label(format!("Worker Model: {}", run.provider));
        if let Some(branch) =
            note_setting_value(&run.notes, coordination::GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY)
        {
            ui.label(format!("Target Branch: {}", branch));
        }
        if let Some(minutes) = note_setting_value(
            &run.notes,
            coordination::GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY,
        ) {
            ui.label(format!("Poll Minutes: {}", minutes));
        }
        return;
    }

    if !coordination::uses_lightweight_runtime_mode(&run.workflow_mode) {
        return;
    }

    if let Some(path) = note_setting_value(&run.notes, "lightweight_task_doc") {
        ui.label(format!("Task List Doc: {}", path));
    }
    if let Some(model) = note_setting_value(&run.notes, "lightweight_task_splitter") {
        ui.label(format!("Task Splitter: {}", model));
    }
    if let Some(model) =
        note_setting_value(&run.notes, coordination::LIGHTWEIGHT_VALIDATION_NOTE_KEY)
    {
        ui.label(format!("Validation Model: {}", model));
    }
    if let Some(models) = note_setting_value(&run.notes, "lightweight_worker_models") {
        ui.label(format!("Worker Models: {}", models));
    }
    ui.label(format!(
        "Repo Configuration: {}",
        coordination::lightweight_repo_configuration_for_run(run).display_name()
    ));
}

fn parse_lightweight_worker_models(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn render_lightweight_worker_model_rows(ui: &mut egui::Ui, worker_models: &mut Vec<String>) {
    if worker_models.is_empty() {
        worker_models.push(ModelProvider::Codex.as_str().to_string());
    }

    let mut add_row = false;
    let can_remove_rows = worker_models.len() > 1;
    let mut remove_index = None;
    for (index, worker_model) in worker_models.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt(("lightweight_worker_model", index))
                .selected_text(worker_model.as_str())
                .show_ui(ui, |ui| {
                    for provider in ModelProvider::ALL {
                        ui.selectable_value(
                            worker_model,
                            provider.as_str().to_string(),
                            provider.display_name(),
                        );
                    }
                });
            if ui.small_button("+").clicked() {
                add_row = true;
            }
            if can_remove_rows && ui.small_button("−").clicked() {
                remove_index = Some(index);
            }
        });
    }

    if let Some(index) = remove_index {
        worker_models.remove(index);
    }
    if add_row {
        worker_models.push(ModelProvider::Codex.as_str().to_string());
    }
}
