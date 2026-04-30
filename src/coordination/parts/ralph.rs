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

pub fn ralph_execute_template_path(run_id: &str) -> String {
    std::env::temp_dir()
        .join(format!("ralph_execute_template_{run_id}.json"))
        .to_string_lossy()
        .into_owned()
}

pub fn ralph_verify_template_path(run_id: &str) -> String {
    std::env::temp_dir()
        .join(format!("ralph_verify_template_{run_id}.json"))
        .to_string_lossy()
        .into_owned()
}

pub fn build_ralph_execute_prompt(run: &CoordinationRun, iteration: u32) -> String {
    let notes = if run.notes.trim().is_empty() {
        "<none>"
    } else {
        run.notes.trim()
    };
    let template_path = ralph_execute_template_path(&run.run_id);
    format!(
        r#"Run Ralph loop iteration {} for this goal:
{}

Completion criteria:
{}

Notes:
{}

Instructions:
- This is one execution pass in an outer verify/retry loop.
- Use the latest Ralph plan artifact as the working checklist for this iteration.
- Inspect the current workspace state before making changes.
- If the criteria are already satisfied, avoid churn and report the evidence.
- If they are not satisfied, make the smallest complete progress you can toward satisfying them.
- Fill in the execution template at {}:
  - Set "outcome" to one of: completed_with_changes, completed_no_change, blocked, partial, failed
  - If outcome is "completed_with_changes", you MUST list the modified files in "changed_files" - this is required!
  - If outcome is "completed_no_change", set "changed_files" to []
  - Set "summary" to a brief description of what was done
  - If blocked or failed, explain in "blockers" or "reason"
 - CRITICAL: If you set outcome to "completed_with_changes" but leave "changed_files" empty or missing, the task will FAIL acceptance checks.
- Do not modify any other fields in the template.
- Output ONLY the filled JSON, no markdown, no explanation."#,
        iteration, run.goal, run.completion_criteria, notes, template_path
    )
}

pub fn build_ralph_plan_prompt(run: &CoordinationRun, iteration: u32) -> String {
    let notes = if run.notes.trim().is_empty() {
        "<none>"
    } else {
        run.notes.trim()
    };
    let lifecycle = if iteration == 1 {
        "Create the initial Ralph working plan before any implementation changes. Make sure the initial plan contains every acceptance criterion from the completion criteria."
    } else {
        "Refresh the existing Ralph working plan using the latest verification outcome and current workspace state. Preserve every acceptance criterion and update statuses/evidence instead of dropping rows."
    };
    format!(
        r#"Create or update the Ralph working plan for iteration {}.

Goal:
{}

Completion criteria:
{}

Notes:
{}

Instructions:
- {}
- Check the current workspace state and the dependency artifacts before writing the plan.
- Return JSON only using the standard task report schema.
- Put markdown only in `details` with these exact sections:
  - `Raw Acceptance Criteria`
  - `Status Board`
  - `Current Focus`
  - `Evidence`
- In `Raw Acceptance Criteria`, copy every completion criterion from the run configuration so the plan keeps the full checklist.
- In `Status Board`, include one row per criterion and mark it as `not_started`, `partial`, `complete`, or `blocked`.
- In `Current Focus`, list only the smallest next actions for items that are not `complete`.
- In `Evidence`, cite concrete files, commands, tests, or dependency artifacts that justify the current statuses.
- Set `changed_files` to [] because this task only updates the plan artifact.
- Use `partial` if any criterion remains incomplete, `completed_no_change` if all are satisfied, and `blocked` only when there is no viable next step."#,
        iteration, run.goal, run.completion_criteria, notes, lifecycle
    )
}

pub fn build_ralph_verify_prompt(run: &CoordinationRun, iteration: u32) -> String {
    let template_path = ralph_verify_template_path(&run.run_id);
    format!(
        r#"Verify whether Ralph loop iteration {} satisfied the goal.

Goal:
{}

Completion criteria:
{}

Instructions:
- Check the actual workspace state and the dependency artifacts.
- Do not trust the prior iteration's claims without inspecting the repo.
- Fill in the verification template at {}:
  - Set "complete" to true only if every criterion is satisfied right now.
  - If "complete" is false, "reason" must explain exactly what remains.
- Do not modify any other fields in the template.
- Output ONLY the filled JSON, no markdown, no explanation."#,
        iteration, run.goal, run.completion_criteria, template_path
    )
}

pub fn ralph_execute_tools(allow_write_tasks: bool) -> Vec<String> {
    let mut tools = vec!["filesystem".to_string(), "search".to_string()];
    if allow_write_tasks {
        tools.push("edit".to_string());
    }
    tools
}

pub fn ralph_execute_write_scope(run_id: &str, allow_write_tasks: bool) -> Vec<String> {
    let template_path = ralph_execute_template_path(run_id);
    if allow_write_tasks {
        vec![".".to_string(), template_path]
    } else {
        vec![template_path]
    }
}

pub fn ralph_verify_tools() -> Vec<String> {
    vec![
        "filesystem".to_string(),
        "search".to_string(),
        "edit".to_string(),
    ]
}

pub fn ralph_verify_read_scope(run_id: &str) -> Vec<String> {
    vec![
        "**".to_string(),
        "artifacts/**".to_string(),
        ralph_verify_template_path(run_id),
    ]
}

pub fn ralph_verify_write_scope(run_id: &str) -> Vec<String> {
    vec![ralph_verify_template_path(run_id)]
}

pub fn parse_ralph_verification(body: &str, run_id: &str) -> Result<RalphVerificationResult> {
    // Prefer the provider response, then fall back to the template file if needed.
    let template_path = ralph_verify_template_path(run_id);
    let json = if let Some(json) = extract_first_json_object(body) {
        Some(json.to_string())
    } else if let Ok(content) = std::fs::read_to_string(&template_path) {
        extract_first_json_object(&content).map(|s| s.to_string())
    } else {
        None
    };
    let json = json.context("missing JSON object in Ralph verification output")?;
    let mut parsed: RalphVerificationResult =
        serde_json::from_str(&json).context("invalid Ralph verification JSON")?;
    parsed.reason = parsed.reason.trim().to_string();
    if parsed.reason.is_empty() {
        parsed.reason = if parsed.complete {
            "Completion criteria satisfied.".to_string()
        } else {
            "Completion criteria not yet satisfied.".to_string()
        };
    }
    Ok(parsed)
}

pub fn parse_ralph_execution(body: &str, run_id: &str) -> Result<RalphExecutionResult> {
    // Prefer the provider response, then fall back to the template file if needed.
    let template_path = ralph_execute_template_path(run_id);
    let json = if let Some(json) = extract_first_json_object(body) {
        Some(json.to_string())
    } else if let Ok(content) = std::fs::read_to_string(&template_path) {
        extract_first_json_object(&content).map(|s| s.to_string())
    } else {
        None
    };
    let json = json.context("missing JSON object in Ralph execution output")?;
    let mut parsed: RalphExecutionResult =
        serde_json::from_str(&json).context("invalid Ralph execution JSON")?;
    parsed.summary = parsed.summary.trim().to_string();

    if parsed.outcome == CoordinationTaskOutcome::CompletedWithChanges
        && (parsed.changed_files.is_empty()
            || parsed.changed_files.iter().all(|s| s.trim().is_empty()))
    {
        anyhow::bail!(
            "invalid Ralph execution JSON: outcome=completed_with_changes requires non-empty changed_files"
        );
    }

    if parsed.summary.is_empty() {
        parsed.summary = match parsed.outcome {
            CoordinationTaskOutcome::CompletedWithChanges => {
                "Made changes to workspace".to_string()
            }
            CoordinationTaskOutcome::CompletedNoChange => "No changes needed".to_string(),
            _ => "Task execution completed".to_string(),
        };
    }
    Ok(parsed)
}

pub fn expand_ralph_iteration(
    run_id: &str,
    verify_task: &CoordinationTask,
    body: &str,
) -> Result<()> {
    let verification = parse_ralph_verification(body, run_id)?;
    let mut run = load_run(run_id)?;
    let current_iteration = verify_task
        .iteration
        .context("Ralph verify task missing iteration metadata")?;

    if verification.complete {
        append_event(
            run_id,
            "ralph_verified",
            Some(&verify_task.task_id),
            &format!(
                "Ralph loop satisfied completion criteria at iteration {}. {}",
                current_iteration, verification.reason
            ),
        )?;
        return Ok(());
    }

    if current_iteration >= run.max_iterations {
        anyhow::bail!(
            "Ralph loop hit the iteration limit ({}) before completion. {}",
            run.max_iterations,
            verification.reason
        );
    }

    let existing_children = load_tasks(run_id)?
        .into_iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(verify_task.task_id.as_str()))
        .count();
    if existing_children > 0 {
        return Ok(());
    }

    let next_iteration = current_iteration + 1;
    let plan_id = Uuid::new_v4().to_string();
    let execute_id = Uuid::new_v4().to_string();
    let verify_id = Uuid::new_v4().to_string();
    let previous_plan_id =
        verify_task
            .dependencies
            .iter()
            .find_map(|task_id| match load_task(run_id, task_id) {
                Ok(task) if task.kind == CoordinationTaskKind::RalphPlan => Some(task.task_id),
                _ => None,
            });

    let mut plan_dependencies = Vec::new();
    if let Some(previous_plan_id) = previous_plan_id {
        plan_dependencies.push(previous_plan_id);
    }
    plan_dependencies.push(verify_task.task_id.clone());

    let mut plan_task = make_task(
        &run,
        &plan_id,
        Some(verify_task.task_id.clone()),
        CoordinationTaskKind::RalphPlan,
        Some(run.provider.clone()),
        format!("Plan Ralph iteration {}", next_iteration),
        build_ralph_plan_prompt(&run, next_iteration),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        plan_dependencies,
        CoordinationTaskStatus::Blocked,
    );
    plan_task.iteration = Some(next_iteration);

    let mut execute_task = make_task(
        &run,
        &execute_id,
        Some(plan_id.clone()),
        CoordinationTaskKind::RalphExecute,
        Some(run.provider.clone()),
        format!("Ralph iteration {}", next_iteration),
        build_ralph_execute_prompt(&run, next_iteration),
        ralph_execute_tools(run.allow_write_tasks),
        vec!["**".to_string(), "artifacts/**".to_string()],
        ralph_execute_write_scope(&run.run_id, run.allow_write_tasks),
        vec![plan_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    execute_task.iteration = Some(next_iteration);

    let mut next_verify_task = make_task(
        &run,
        &verify_id,
        Some(execute_id.clone()),
        CoordinationTaskKind::RalphVerify,
        Some(run.provider.clone()),
        format!("Verify Ralph iteration {}", next_iteration),
        build_ralph_verify_prompt(&run, next_iteration),
        ralph_verify_tools(),
        ralph_verify_read_scope(&run.run_id),
        ralph_verify_write_scope(&run.run_id),
        vec![plan_id.clone(), execute_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    next_verify_task.iteration = Some(next_iteration);

    write_json_atomic(&task_path(run_id, &plan_id), &plan_task)?;
    write_json_atomic(&task_path(run_id, &execute_id), &execute_task)?;
    write_json_atomic(&task_path(run_id, &verify_id), &next_verify_task)?;

    run.task_ids.push(plan_id.clone());
    run.task_ids.push(execute_id.clone());
    run.task_ids.push(verify_id.clone());
    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;

    append_event(
        run_id,
        "ralph_continue",
        Some(&verify_task.task_id),
        &format!(
            "Ralph iteration {} incomplete. {} Queued plan refresh and iteration {}.",
            current_iteration, verification.reason, next_iteration
        ),
    )?;
    Ok(())
}

pub fn derive_run_status(
    tasks: &[CoordinationTask],
    completion_gate_issues: &[String],
) -> CoordinationRunStatus {
    if tasks.is_empty() {
        return CoordinationRunStatus::Planning;
    }
    if tasks.iter().any(|task| {
        !matches!(
            task.status,
            CoordinationTaskStatus::Completed
                | CoordinationTaskStatus::Failed
                | CoordinationTaskStatus::Canceled
        ) && task
            .recovery_action
            .is_some_and(|action| action != TaskRecoveryAction::None)
    }) {
        return CoordinationRunStatus::Recovering;
    }
    if tasks
        .iter()
        .any(|task| task.status == CoordinationTaskStatus::Failed)
    {
        return CoordinationRunStatus::Failed;
    }
    if tasks
        .iter()
        .all(|task| task.status == CoordinationTaskStatus::Completed)
    {
        if completion_gate_issues.is_empty() {
            return CoordinationRunStatus::Completed;
        }
        return CoordinationRunStatus::Failed;
    }
    if tasks.iter().any(|task| {
        matches!(
            task.status,
            CoordinationTaskStatus::Queued
                | CoordinationTaskStatus::Running
                | CoordinationTaskStatus::Claiming
                | CoordinationTaskStatus::ArtifactReady
        )
    }) {
        return CoordinationRunStatus::Active;
    }
    CoordinationRunStatus::WaitingForArtifacts
}

pub fn merge_derived_run_status(
    current_status: &CoordinationRunStatus,
    derived_status: CoordinationRunStatus,
) -> CoordinationRunStatus {
    if *current_status == CoordinationRunStatus::Canceled {
        CoordinationRunStatus::Canceled
    } else {
        derived_status
    }
}

pub fn run_completion_gate_issues(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
) -> Result<Vec<String>> {
    let mut issues = Vec::new();

    for task in tasks {
        let artifact_path = artifact_result_path(&run.run_id, &task.task_id);
        if !artifact_path.is_file() {
            if !completed_task_has_acceptable_artifact_fallback(task) {
                issues.push(format!("missing artifact for {}", task.title));
            }
            continue;
        }

        let raw = fs::read_to_string(&artifact_path)
            .with_context(|| format!("failed to read {}", artifact_path.display()))?;
        let artifact: TaskArtifact = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", artifact_path.display()))?;

        if !artifact.report.validation_errors.is_empty() {
            issues.push(format!("{} has acceptance failures", task.title));
        }

        let superseded_partial = artifact.report.outcome == CoordinationTaskOutcome::Partial
            && (superseded_partial_planning_task(tasks, task)
                || (uses_lightweight_runtime_mode(&run.workflow_mode)
                    && lightweight_review_task_is_superseded(tasks, task))
                || superseded_partial_lightweight_publish_task(run, tasks, task)
                || superseded_partial_lightweight_write_task(run, tasks, task)
                || superseded_partial_branch_fix_write_task(run, tasks, task));

        if !superseded_partial
            && matches!(
                artifact.report.outcome,
                CoordinationTaskOutcome::Blocked
                    | CoordinationTaskOutcome::Partial
                    | CoordinationTaskOutcome::Failed
            )
        {
            issues.push(format!(
                "{} ended with outcome {}",
                task.title,
                artifact.report.outcome.as_str()
            ));
        }
    }

    Ok(issues)
}

pub(crate) fn completed_task_has_acceptable_artifact_fallback(task: &CoordinationTask) -> bool {
    task.status == CoordinationTaskStatus::Completed
        && task
            .artifact_summary
            .as_deref()
            .map(|summary| !summary.trim().is_empty())
            .unwrap_or(false)
}

fn superseded_partial_planning_task(tasks: &[CoordinationTask], task: &CoordinationTask) -> bool {
    matches!(
        task.kind,
        CoordinationTaskKind::PlanFlatTasks | CoordinationTaskKind::PlanCodeReview
    ) && tasks
        .iter()
        .any(|candidate| candidate.parent_task_id.as_deref() == Some(task.task_id.as_str()))
}

fn superseded_partial_branch_fix_write_task(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
    task: &CoordinationTask,
) -> bool {
    run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
        && tasks.iter().any(|candidate| {
            candidate.kind == CoordinationTaskKind::ReviewArtifacts
                && candidate
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == &task.task_id)
        })
}

fn superseded_partial_lightweight_publish_task(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
    task: &CoordinationTask,
) -> bool {
    uses_lightweight_runtime_mode(&run.workflow_mode)
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
        && task
            .artifact_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("Deferred git publish failed:"))
        && tasks.iter().any(|candidate| {
            candidate.status == CoordinationTaskStatus::Completed
                && candidate.title == "Repair repository after failed run"
                && candidate
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == &task.task_id)
        })
}

fn superseded_partial_lightweight_write_task(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
    task: &CoordinationTask,
) -> bool {
    uses_lightweight_runtime_mode(&run.workflow_mode)
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
        && tasks.iter().any(|candidate| {
            candidate.kind == CoordinationTaskKind::ReviewArtifacts
                && candidate
                    .dependencies
                    .iter()
                    .any(|dependency| dependency == &task.task_id)
                && review_chain_has_completed_no_change(tasks, &candidate.task_id)
        })
}

fn review_chain_has_completed_no_change(tasks: &[CoordinationTask], review_task_id: &str) -> bool {
    let mut stack = vec![review_task_id.to_string()];
    let mut seen = BTreeSet::new();

    while let Some(task_id) = stack.pop() {
        if !seen.insert(task_id.clone()) {
            continue;
        }

        let Some(task) = tasks.iter().find(|candidate| candidate.task_id == task_id) else {
            continue;
        };

        if task.kind == CoordinationTaskKind::ReviewArtifacts
            && task.status == CoordinationTaskStatus::Completed
            && task
                .artifact_summary
                .as_deref()
                .is_some_and(|summary| summary.contains("[completed_no_change]"))
        {
            return true;
        }

        for child in tasks.iter().filter(|candidate| {
            candidate.kind == CoordinationTaskKind::ReviewArtifacts
                && candidate.parent_task_id.as_deref() == Some(task.task_id.as_str())
        }) {
            stack.push(child.task_id.clone());
        }
    }

    false
}

pub fn completed_task_generations(
    tasks: &[CoordinationTask],
) -> std::collections::HashMap<String, u32> {
    tasks
        .iter()
        .filter(|task| task.status == CoordinationTaskStatus::Completed)
        .map(|task| (task.task_id.clone(), effective_dependency_generation(task)))
        .collect()
}

pub fn task_dependencies_completed(
    task: &CoordinationTask,
    completed: &std::collections::HashMap<String, u32>,
) -> bool {
    task.dependencies.iter().all(|dependency| {
        completed.get(dependency).copied() == Some(effective_dependency_generation(task))
    })
}

fn completed_dependency_generation(
    task: &CoordinationTask,
    completed: &std::collections::HashMap<String, u32>,
) -> Option<u32> {
    let mut generations = task
        .dependencies
        .iter()
        .map(|dependency| completed.get(dependency).copied())
        .collect::<Option<Vec<u32>>>()?
        .into_iter();
    let first = generations
        .next()
        .unwrap_or(effective_dependency_generation(task));
    generations
        .all(|generation| generation == first)
        .then_some(first)
}

pub fn unblock_runnable_task_ids(tasks: &mut [CoordinationTask]) -> Vec<String> {
    let completed = completed_task_generations(tasks);
    let mut changed_task_ids = Vec::new();

    for task in tasks.iter_mut() {
        if let Some(dependency_generation) = completed_dependency_generation(task, &completed) {
            if dependency_generation != effective_dependency_generation(task)
                && matches!(
                    task.status,
                    CoordinationTaskStatus::Blocked | CoordinationTaskStatus::Queued
                )
            {
                task.dependency_generation = Some(dependency_generation);
                task.updated_at = Utc::now();
                changed_task_ids.push(task.task_id.clone());
            }
        }
        let dependencies_completed = task_dependencies_completed(task, &completed);
        match task.status {
            CoordinationTaskStatus::Blocked if dependencies_completed => {
                task.status = CoordinationTaskStatus::Queued;
                task.updated_at = Utc::now();
                if !changed_task_ids.contains(&task.task_id) {
                    changed_task_ids.push(task.task_id.clone());
                }
            }
            CoordinationTaskStatus::Queued if !dependencies_completed => {
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                if !changed_task_ids.contains(&task.task_id) {
                    changed_task_ids.push(task.task_id.clone());
                }
            }
            _ => {}
        }
    }

    changed_task_ids
}

pub fn unblock_runnable_tasks(tasks: &mut [CoordinationTask]) -> bool {
    !unblock_runnable_task_ids(tasks).is_empty()
}
