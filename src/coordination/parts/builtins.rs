use super::*;
#[allow(unused_imports)]
use crate::config;
#[allow(unused_imports)]
use crate::data::ChatSession;
#[allow(unused_imports)]
use crate::memory;
#[allow(unused_imports)]
use crate::model::ModelProvider;
#[allow(unused_imports)]
use crate::provider_bridge::{
    autonomous_provider_overrides, call_provider_cli_stream_with_overrides,
};
#[allow(unused_imports)]
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
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
use std::sync::{mpsc, Arc};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;

const SURVEY_WORKSPACE_LIMIT: usize = 200;

#[derive(Deserialize)]
pub struct SurveyWorkspaceSnapshot {
    directories: Vec<String>,
    files: Vec<String>,
}

fn survey_workspace_summary(
    task: &CoordinationTask,
    dirs: usize,
    files: usize,
    via_runsc: bool,
) -> String {
    let execution_mode = if via_runsc { " via runsc" } else { "" };
    format!(
        "Surveyed workspace{} at {} ({} dirs, {} files sampled).",
        execution_mode, task.cwd, dirs, files
    )
}

pub fn collect_survey_workspace_entries(
    task: &CoordinationTask,
    run: &CoordinationRun,
) -> Result<(Vec<String>, Vec<String>)> {
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        return collect_survey_workspace_entries_runsc(task, run);
    }

    let mut dirs = Vec::new();
    let mut files = Vec::new();
    collect_workspace_entries(
        Path::new(&task.cwd),
        Path::new(&task.cwd),
        &mut dirs,
        &mut files,
        SURVEY_WORKSPACE_LIMIT,
    )?;
    Ok((dirs, files))
}

pub fn collect_survey_workspace_entries_runsc(
    task: &CoordinationTask,
    run: &CoordinationRun,
) -> Result<(Vec<String>, Vec<String>)> {
    let image = config::background_runsc_image();
    let effective_includes = effective_runsc_workspace_includes(run, Some(task));
    let _runsc_env_guard = AutonomousRunscEnvGuard::activate(run)?;
    let script = r#"
import json
from pathlib import Path

LIMIT = 200
SKIP_DIRS = {".git", "target", "node_modules", "dist"}

root = Path(".")
directories = []
files = []

def walk(current: Path) -> None:
    if len(directories) + len(files) >= LIMIT:
        return
    try:
        entries = sorted(current.iterdir(), key=lambda path: path.name)
    except OSError:
        return
    for entry in entries:
        if len(directories) + len(files) >= LIMIT:
            break
        rel = entry.relative_to(root).as_posix()
        try:
            if entry.is_dir():
                if entry.name in SKIP_DIRS:
                    continue
                directories.append(rel)
                walk(entry)
            elif entry.is_file():
                files.append(rel)
        except OSError:
            continue

walk(root)
print(json.dumps({"directories": directories, "files": files}))
"#;

    let mut command = std::process::Command::new("python3");
    command.arg("-c").arg(script).current_dir(&task.cwd);
    apply_autonomous_runsc_env_to_command(&mut command, run)?;
    let _workspace_sync = ai_providers::sandbox::wrap_command_with_runsc(
        &mut command,
        Some(image.as_str()),
        &effective_includes,
        &[],
    )?;
    let output = command
        .output()
        .with_context(|| format!("failed to execute runsc workspace survey for {}", task.cwd))?;
    if !output.status.success() {
        anyhow::bail!(
            "runsc workspace survey failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let snapshot: SurveyWorkspaceSnapshot =
        serde_json::from_slice(&output.stdout).context("failed to parse runsc workspace survey")?;
    Ok((snapshot.directories, snapshot.files))
}

fn run_code_review_automated_checks_runsc(
    task: &CoordinationTask,
    run: &CoordinationRun,
) -> Result<Vec<code_review::AutomatedToolCheck>> {
    let commands = [
        ("cargo check", vec!["check"]),
        (
            "cargo clippy -- -D warnings",
            vec!["clippy", "--", "-D", "warnings"],
        ),
        ("cargo test", vec!["test"]),
        ("cargo fmt --check", vec!["fmt", "--check"]),
    ];
    commands
        .into_iter()
        .map(|(label, args)| run_one_code_review_check_runsc(task, run, label, &args))
        .collect()
}

fn run_one_code_review_check_runsc(
    task: &CoordinationTask,
    run: &CoordinationRun,
    label: &str,
    args: &[&str],
) -> Result<code_review::AutomatedToolCheck> {
    let image = config::background_runsc_image();
    let _runsc_env_guard = AutonomousRunscEnvGuard::activate(run)?;
    let mut command = std::process::Command::new("cargo");
    command.args(args).current_dir(&task.cwd);
    apply_autonomous_runsc_env_to_command(&mut command, run)?;
    let _workspace_sync = ai_providers::sandbox::wrap_command_with_runsc(
        &mut command,
        Some(image.as_str()),
        &effective_runsc_workspace_includes(run, Some(task)),
        &[],
    )?;
    let output = command.output().with_context(|| {
        format!(
            "failed to execute runsc code review automated check `{}` for {}",
            label, task.cwd
        )
    })?;
    Ok(summarize_code_review_check_output(label, output))
}

fn summarize_code_review_check_output(
    label: &str,
    output: std::process::Output,
) -> code_review::AutomatedToolCheck {
    let mut combined = String::new();
    if !output.stdout.is_empty() {
        combined.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    if !output.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let significant_lines: Vec<&str> = combined
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with('=')
                && !trimmed.starts_with('|')
                && !trimmed.starts_with("-->")
                && !trimmed.starts_with("---")
        })
        .take(3)
        .collect();
    let summary = if significant_lines.is_empty() {
        if output.status.success() {
            "ok".to_string()
        } else {
            "command failed".to_string()
        }
    } else {
        significant_lines.join("; ")
    };
    code_review::AutomatedToolCheck {
        command: label.to_string(),
        status: if output.status.success() {
            "passed".to_string()
        } else {
            "failed".to_string()
        },
        summary,
    }
}

pub fn execute_survey_workspace(task: &CoordinationTask) -> Result<TaskArtifact> {
    let run = load_run(&task.run_id)?;
    ensure_task_workspace_exists(&run, task, task.assigned_provider.as_deref())?;
    let (dirs, files) = collect_survey_workspace_entries(task, &run)?;
    if run.workflow_mode == AutonomousWorkflowMode::CodeReview {
        let automated_checks = if run.execution_environment == AutonomousExecutionEnvironment::Runsc
        {
            run_code_review_automated_checks_runsc(task, &run)?
        } else {
            code_review::run_automated_checks(Path::new(&task.cwd))
        };
        let body = serde_json::to_string_pretty(&serde_json::json!({
            "directories": dirs,
            "files": files,
            "automated_checks": automated_checks,
        }))?;
        return Ok(TaskArtifact {
            run_id: task.run_id.clone(),
            task_id: task.task_id.clone(),
            provider: task.assigned_provider.clone(),
            task_category: task.task_category,
            fallback_from: task.provider_hint.clone(),
            summary: format!(
                "Surveyed review workspace at {} and captured {} automated checks.",
                task.cwd,
                automated_checks.len()
            ),
            body: body.clone(),
            completed_at: Utc::now(),
            report: TaskExecutionReport {
                outcome: CoordinationTaskOutcome::CompletedNoChange,
                summary: format!(
                    "Surveyed review workspace at {} and captured {} automated checks.",
                    task.cwd,
                    automated_checks.len()
                ),
                details: body,
                tests_run: automated_checks
                    .iter()
                    .map(|check| check.command.clone())
                    .collect(),
                design_requirements_satisfied: vec![
                    "Collected current workspace context.".to_string(),
                    "Captured automated tool results for the review plan.".to_string(),
                ],
                ..Default::default()
            },
        });
    }
    let body = format!(
        "# Workspace Survey\n\nDirectories:\n{}\n\nFiles:\n{}",
        format_lines(&dirs),
        format_lines(&files)
    );
    let summary = survey_workspace_summary(
        task,
        dirs.len(),
        files.len(),
        run.execution_environment == AutonomousExecutionEnvironment::Runsc,
    );
    Ok(TaskArtifact {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: summary.clone(),
        body,
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary,
            details: format!(
                "# Workspace Survey\n\nDirectories:\n{}\n\nFiles:\n{}",
                format_lines(&dirs),
                format_lines(&files)
            ),
            design_requirements_satisfied: vec![
                "Collected current workspace context.".to_string(),
                "Captured a bounded sample of files and directories.".to_string(),
            ],
            ..Default::default()
        },
    })
}

pub fn execute_plan_code_review(task: &CoordinationTask) -> Result<TaskArtifact> {
    let run = load_run(&task.run_id)?;
    let scope = run.review_scope.clone().unwrap_or_default();
    let automated_checks = load_review_automated_checks(&task.run_id, &task.dependencies);
    let profile_warnings = code_review::review_profile_warnings(Path::new(&task.cwd), Utc::now());
    let mut plan = code_review::build_review_plan(
        Path::new(&task.cwd),
        &scope,
        run.review_dimensions.as_deref(),
        run.review_fix_mode.clone().unwrap_or_default(),
        automated_checks.clone(),
        profile_warnings.clone(),
        Utc::now(),
    )?;
    plan.output_path = run.design_doc_path.clone();
    let body = serde_json::to_string_pretty(&plan)?;
    Ok(TaskArtifact {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: format!("Planned {} code review chunk(s).", plan.review_tasks.len()),
        body: body.clone(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: format!("Planned {} code review chunk(s).", plan.review_tasks.len()),
            details: body,
            tests_run: automated_checks
                .iter()
                .map(|check| check.command.clone())
                .collect(),
            warnings: profile_warnings,
            design_requirements_satisfied: vec![
                "Validated review profile freshness at plan time.".to_string(),
                "Planned read-only review chunks from the requested review scope.".to_string(),
            ],
            ..Default::default()
        },
    })
}

pub fn execute_code_review_materialization(task: &CoordinationTask) -> Result<TaskArtifact> {
    let metadata = load_review_task_metadata(&task.run_id, &task.task_id)
        .context("review materialization task is missing review metadata")?;
    let output_path = metadata
        .output_path
        .as_ref()
        .context("review materialization task is missing output_path")?;
    let artifacts = load_review_dependency_artifacts(&task.run_id, &task.dependencies)?;
    let workspace_name = Path::new(&task.cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    let document = code_review::render_review_document(
        workspace_name,
        Utc::now().date_naive(),
        &metadata.scope_description,
        &metadata,
        &artifacts,
    );
    let candidate = Path::new(&task.cwd).join(output_path);
    let parent = candidate
        .parent()
        .context("review output path missing parent")?;
    fs::create_dir_all(parent)?;
    fs::write(&candidate, &document)
        .with_context(|| format!("failed to write {}", candidate.display()))?;
    Ok(TaskArtifact {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: format!("Materialized code review document at {}.", output_path),
        body: document.clone(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: format!("Materialized code review document at {}.", output_path),
            details: format!(
                "Wrote the aggregated code review document to `{}`.",
                output_path
            ),
            changed_files: vec![output_path.clone()],
            tests_run: metadata
                .automated_checks
                .iter()
                .map(|check| check.command.clone())
                .collect(),
            warnings: metadata.profile_warnings.clone(),
            design_requirements_satisfied: vec![
                format!("Wrote the review document to {}.", output_path),
                "Ordered findings by severity with file:line references.".to_string(),
            ],
            ..Default::default()
        },
    })
}

pub fn is_provider_backed(task: &CoordinationTask) -> bool {
    if task.kind == CoordinationTaskKind::PlanCodeReview {
        return false;
    }
    if task.kind == CoordinationTaskKind::MaterializeDesignDoc
        && load_review_task_metadata(&task.run_id, &task.task_id).is_some()
    {
        return false;
    }
    matches!(
        task.kind,
        CoordinationTaskKind::DraftDesignDoc
            | CoordinationTaskKind::ConvergeDesignDoc
            | CoordinationTaskKind::MaterializeDesignDoc
            | CoordinationTaskKind::PlanFlatTasks
            | CoordinationTaskKind::RalphPlan
            | CoordinationTaskKind::GeneratedReadTask
            | CoordinationTaskKind::GeneratedWriteTask
            | CoordinationTaskKind::ReviewArtifacts
            | CoordinationTaskKind::ApplyApprovedEdits
            | CoordinationTaskKind::RalphExecute
            | CoordinationTaskKind::RalphVerify
            | CoordinationTaskKind::PerfBaseline
            | CoordinationTaskKind::PerfParse
            | CoordinationTaskKind::PerfApply
            | CoordinationTaskKind::PerfBench
            | CoordinationTaskKind::PerfEvaluate
    )
}

pub fn load_review_automated_checks(
    run_id: &str,
    dependencies: &[String],
) -> Vec<code_review::AutomatedToolCheck> {
    for dependency in dependencies {
        let path = artifact_result_path(run_id, dependency);
        let Ok(raw) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(artifact) = serde_json::from_str::<TaskArtifact>(&raw) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&artifact.body) else {
            continue;
        };
        let Some(checks) = value.get("automated_checks") else {
            continue;
        };
        if let Ok(parsed) =
            serde_json::from_value::<Vec<code_review::AutomatedToolCheck>>(checks.clone())
        {
            return parsed;
        }
    }
    Vec::new()
}

pub fn load_review_dependency_artifacts(
    run_id: &str,
    dependencies: &[String],
) -> Result<Vec<code_review::ReviewArtifactResponse>> {
    let mut artifacts = Vec::new();
    for dependency in dependencies {
        let path = artifact_result_path(run_id, dependency);
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let artifact: TaskArtifact = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let parsed: code_review::ReviewArtifactResponse = serde_json::from_str(&artifact.body)
            .with_context(|| {
                format!(
                    "failed to parse review artifact body from {}",
                    path.display()
                )
            })?;
        artifacts.push(parsed);
    }
    Ok(artifacts)
}
