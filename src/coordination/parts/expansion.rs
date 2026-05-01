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

pub fn expand_plan_task(run_id: &str, plan_task: &CoordinationTask, body: &str) -> Result<()> {
    let run = load_run(run_id)?;
    let specs = parse_generated_task_specs(body, Some(&run)).with_context(|| {
        format!(
            "failed to parse generated tasks from plan {}",
            plan_task.task_id
        )
    })?;
    validate_generated_task_specs(&specs)?;
    if specs.is_empty() {
        anyhow::bail!("plan task did not produce any generated tasks");
    }

    let mut run = run;
    let mut tasks = load_tasks(run_id)?;
    let existing_generated: Vec<String> = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .filter(|task| is_generated_task_kind(&task.kind))
        .map(|task| task.task_id.clone())
        .collect();
    if !existing_generated.is_empty() {
        rewire_follow_up_tasks(run_id, &plan_task.task_id, &mut tasks, &existing_generated)?;
        return Ok(());
    }

    let now = Utc::now();
    let mut generated_ids = Vec::new();
    let mut generated_specs = Vec::new();
    for (index, spec) in specs.iter().cloned().enumerate() {
        let kind = parse_generated_kind(&spec.kind)?;
        if kind == CoordinationTaskKind::GeneratedWriteTask
            && (!run.allow_write_tasks || spec.write_scope.is_empty())
        {
            append_event(
                run_id,
                "task_skipped",
                Some(&plan_task.task_id),
                &format!(
                    "Skipped generated write task '{}' because write tasks are disabled or write_scope was empty.",
                    spec.title
                ),
            )?;
            continue;
        }

        let task_id = Uuid::new_v4().to_string();
        generated_specs.push((task_id.clone(), spec.clone()));
        let assigned_provider = if uses_lightweight_runtime_mode(&run.workflow_mode)
            && plan_task.task_category == Some(TaskCategory::Verify)
        {
            Some(lightweight_task_splitter_provider(&run))
        } else {
            resolve_generated_task_provider(&run, spec.provider.as_deref(), index)
        };
        let prompt = if uses_lightweight_runtime_mode(&run.workflow_mode) {
            lightweight_generated_task_prompt(&run, &kind, &spec.prompt)
        } else {
            spec.prompt
        };
        let task = CoordinationTask {
            provider_hint: None,
            required_role: None,
            task_category: default_task_category_for_kind(&kind),
            task_id: task_id.clone(),
            run_id: run_id.to_string(),
            parent_task_id: Some(plan_task.task_id.clone()),
            kind: kind.clone(),
            assigned_provider,
            iteration: None,
            title: spec.title,
            prompt,
            cwd: run.workspace_path.clone(),
            allowed_tools: normalize_allowed_tools(&kind, spec.allowed_tools),
            read_scope: normalize_read_scope(&kind, spec.read_scope),
            write_scope: normalize_write_scope(&kind, spec.write_scope),
            acceptance_checks: normalize_acceptance_checks(&kind, spec.acceptance_checks),
            review_metadata: None,
            dependencies: vec![plan_task.task_id.clone()],
            execution_generation: plan_task.execution_generation,
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: default_task_max_attempts(),
            publish_attempt_count: 0,
            estimated_minutes: Some(normalize_task_estimated_minutes(
                Some(spec.estimated_minutes),
                &kind,
            )),
            status: CoordinationTaskStatus::Blocked,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: now,
            updated_at: now,
        };
        write_json_atomic(&task_path(run_id, &task_id), &task)?;
        tasks.push(task.clone());
        run.task_ids.push(task_id.clone());
        generated_ids.push(task_id.clone());
        append_event(
            run_id,
            "task_generated",
            Some(&task_id),
            &format!(
                "Generated {} task '{}'.",
                generated_task_label(&kind),
                task.title
            ),
        )?;
    }

    if generated_ids.is_empty() {
        anyhow::bail!("plan task did not yield any runnable generated tasks");
    }

    apply_generated_task_dependencies(&run, &plan_task.task_id, &generated_specs, &mut tasks);
    for task_id in &generated_ids {
        if let Some(task) = tasks.iter().find(|task| task.task_id == *task_id) {
            write_json_atomic(&task_path(run_id, task_id), task)?;
        }
    }

    rewire_follow_up_tasks(run_id, &plan_task.task_id, &mut tasks, &generated_ids)?;
    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;
    Ok(())
}

pub fn expand_code_review_plan(
    run_id: &str,
    plan_task: &CoordinationTask,
    body: &str,
) -> Result<()> {
    let plan: code_review::CodeReviewPlanArtifact = serde_json::from_str(body)
        .with_context(|| format!("failed to parse code review plan {}", plan_task.task_id))?;
    if plan.review_tasks.is_empty() {
        anyhow::bail!("code review plan did not produce any review tasks");
    }

    let mut run = load_run(run_id)?;
    let mut tasks = load_tasks(run_id)?;
    let now = Utc::now();
    let mut generated_ids = Vec::new();
    for (index, review_plan) in plan.review_tasks.iter().enumerate() {
        if let Some(task) = tasks.iter_mut().find(|task| {
            task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
                && task.title == review_plan.title
        }) {
            task.assigned_provider = resolve_generated_task_provider(&run, None, index);
            task.task_category =
                default_task_category_for_kind(&CoordinationTaskKind::ReviewArtifacts);
            task.prompt = review_plan.prompt.clone();
            task.cwd = run.workspace_path.clone();
            task.allowed_tools = vec!["filesystem".to_string(), "search".to_string()];
            task.read_scope = review_plan.read_scope.clone();
            task.write_scope.clear();
            task.acceptance_checks = vec![
                "The review inspects the current workspace files in scope.".to_string(),
                "Findings cite file:line references and stay within the findings schema."
                    .to_string(),
            ];
            task.dependencies = vec![plan_task.task_id.clone()];
            task.estimated_minutes = Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::ReviewArtifacts,
            ));
            task.status = CoordinationTaskStatus::Blocked;
            task.updated_at = now;
            write_json_atomic(&task_path(run_id, &task.task_id), task)?;
            if let Some(metadata) = review_plan.metadata.as_ref() {
                write_review_task_metadata(run_id, &task.task_id, metadata)?;
            }
            generated_ids.push(task.task_id.clone());
            continue;
        }

        let task_id = Uuid::new_v4().to_string();
        let review_metadata = review_plan.metadata.clone();
        let task = CoordinationTask {
            provider_hint: None,
            required_role: None,
            task_category: default_task_category_for_kind(&CoordinationTaskKind::ReviewArtifacts),
            task_id: task_id.clone(),
            run_id: run_id.to_string(),
            parent_task_id: Some(plan_task.task_id.clone()),
            kind: CoordinationTaskKind::ReviewArtifacts,
            assigned_provider: resolve_generated_task_provider(&run, None, index),
            iteration: None,
            title: review_plan.title.clone(),
            prompt: review_plan.prompt.clone(),
            cwd: run.workspace_path.clone(),
            allowed_tools: vec!["filesystem".to_string(), "search".to_string()],
            read_scope: review_plan.read_scope.clone(),
            write_scope: Vec::new(),
            acceptance_checks: vec![
                "The review inspects the current workspace files in scope.".to_string(),
                "Findings cite file:line references and stay within the findings schema."
                    .to_string(),
            ],
            review_metadata: None,
            dependencies: vec![plan_task.task_id.clone()],
            execution_generation: plan_task.execution_generation,
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: default_task_max_attempts(),
            publish_attempt_count: 0,
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::ReviewArtifacts,
            )),
            status: CoordinationTaskStatus::Blocked,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: now,
            updated_at: now,
        };
        write_json_atomic(&task_path(run_id, &task_id), &task)?;
        if let Some(metadata) = review_metadata.as_ref() {
            write_review_task_metadata(run_id, &task_id, metadata)?;
        }
        run.task_ids.push(task_id.clone());
        generated_ids.push(task_id.clone());
        append_event(
            run_id,
            "task_generated",
            Some(&task_id),
            &format!("Generated review task '{}'.", task.title),
        )?;
    }

    let existing_review_children: Vec<String> = tasks
        .iter()
        .filter(|task| {
            task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str())
                && task.kind == CoordinationTaskKind::ReviewArtifacts
        })
        .map(|task| task.task_id.clone())
        .collect();
    let materialize_source_ids = existing_review_children
        .iter()
        .chain(generated_ids.iter())
        .cloned()
        .collect::<Vec<_>>();
    rewire_follow_up_tasks(run_id, &plan_task.task_id, &mut tasks, &generated_ids)?;
    for task in tasks.iter_mut() {
        if task.kind != CoordinationTaskKind::MaterializeDesignDoc {
            continue;
        }
        let depends_on_review_plan = task.dependencies.iter().any(|dependency| {
            dependency == &plan_task.task_id
                || materialize_source_ids
                    .iter()
                    .any(|generated_id| generated_id == dependency)
        });
        if !depends_on_review_plan {
            continue;
        }
        task.dependencies = generated_ids.clone();
        if let Some(mut metadata) = load_review_task_metadata(run_id, &task.task_id) {
            metadata.automated_checks = plan.automated_checks.clone();
            metadata.profile_warnings = plan.profile_warnings.clone();
            metadata.output_path = Some(plan.output_path.clone());
            write_review_task_metadata(run_id, &task.task_id, &metadata)?;
        }
        task.status = CoordinationTaskStatus::Blocked;
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
    }

    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;
    Ok(())
}

pub fn expand_review_task(
    run_id: &str,
    review_task: &CoordinationTask,
    artifact: &TaskArtifact,
) -> Result<()> {
    // GithubBranchFix CI gate: when Partial outcome with no fix_tasks, we may still
    // need to synthesize a wait task if CI failed. Check this before the early return.
    let needs_wait_task_for_ci_failure = {
        let run = load_run(run_id)?;
        if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix
            && artifact.report.outcome == CoordinationTaskOutcome::Partial
            && artifact.report.fix_tasks.is_empty()
        {
            match github_branch_fix_ci_status(&run) {
                Ok(GithubBranchFixCiStatus::Success) => false,
                Ok(GithubBranchFixCiStatus::InProgress) => false,
                Ok(GithubBranchFixCiStatus::Failed)
                | Ok(GithubBranchFixCiStatus::Unknown)
                | Err(_) => true,
            }
        } else {
            false
        }
    };

    if artifact.report.fix_tasks.is_empty() && !needs_wait_task_for_ci_failure {
        return Ok(());
    }

    let mut run = load_run(run_id)?;
    if run.workflow_mode == AutonomousWorkflowMode::LightweightImplementation
        && !lightweight_review_should_recurse(&run, review_task, artifact)
    {
        let reason = if !lightweight_review_has_actionable_repo_local_work(artifact) {
            "suppressed recursive follow-up because the review only emitted non-actionable repo-local next steps"
        } else {
            "suppressed recursive follow-up because the review repeated the same blocker signature as the previous review"
        };
        append_event(
            run_id,
            "review_recursion_suppressed",
            Some(&review_task.task_id),
            &format!("{}.", reason),
        )?;
        return Ok(());
    }
    if !artifact.report.fix_tasks.is_empty() {
        validate_generated_task_specs(&artifact.report.fix_tasks)?;
    }

    let mut tasks = load_tasks(run_id)?;
    let now = Utc::now();
    let mut generated_ids = Vec::new();
    let mut materialized_paths = Vec::new();
    let mut pending_fix_specs = Vec::new();
    for (index, spec) in artifact.report.fix_tasks.iter().cloned().enumerate() {
        if let Some(path) = try_materialize_review_fix_task(run_id, review_task, &spec)? {
            materialized_paths.push(path);
            continue;
        }
        pending_fix_specs.push((index, spec));
    }
    for (index, spec) in pending_fix_specs {
        let kind = parse_generated_kind(&spec.kind)?;
        if kind == CoordinationTaskKind::GeneratedWriteTask && !run.allow_write_tasks {
            anyhow::bail!(
                "review generated write fix task '{}' while write tasks are disabled",
                spec.title
            );
        }
        if let Some(task) = tasks.iter_mut().find(|task| {
            task.parent_task_id.as_deref() == Some(review_task.task_id.as_str())
                && task.kind == kind
                && task.title == spec.title
        }) {
            task.assigned_provider =
                resolve_generated_task_provider(&run, spec.provider.as_deref(), index);
            task.task_category = default_task_category_for_kind(&kind);
            task.prompt = if uses_lightweight_runtime_mode(&run.workflow_mode) {
                lightweight_generated_task_prompt(&run, &kind, &spec.prompt)
            } else {
                spec.prompt.clone()
            };
            task.cwd = run.workspace_path.clone();
            task.allowed_tools = normalize_allowed_tools(&kind, spec.allowed_tools.clone());
            task.read_scope = normalize_read_scope(&kind, spec.read_scope.clone());
            task.write_scope = normalize_write_scope(&kind, spec.write_scope.clone());
            task.acceptance_checks =
                normalize_acceptance_checks(&kind, spec.acceptance_checks.clone());
            task.dependencies = vec![review_task.task_id.clone()];
            task.estimated_minutes = Some(normalize_task_estimated_minutes(
                Some(spec.estimated_minutes),
                &kind,
            ));
            task.status = CoordinationTaskStatus::Blocked;
            task.updated_at = now;
            write_json_atomic(&task_path(run_id, &task.task_id), task)?;
            generated_ids.push(task.task_id.clone());
            continue;
        }

        let task_id = Uuid::new_v4().to_string();
        let task = CoordinationTask {
            provider_hint: None,
            required_role: None,
            task_category: default_task_category_for_kind(&kind),
            task_id: task_id.clone(),
            run_id: run_id.to_string(),
            parent_task_id: Some(review_task.task_id.clone()),
            kind: kind.clone(),
            assigned_provider: resolve_generated_task_provider(
                &run,
                spec.provider.as_deref(),
                index,
            ),
            iteration: None,
            title: spec.title,
            prompt: if uses_lightweight_runtime_mode(&run.workflow_mode) {
                lightweight_generated_task_prompt(&run, &kind, &spec.prompt)
            } else {
                spec.prompt
            },
            cwd: run.workspace_path.clone(),
            allowed_tools: normalize_allowed_tools(&kind, spec.allowed_tools),
            read_scope: normalize_read_scope(&kind, spec.read_scope),
            write_scope: normalize_write_scope(&kind, spec.write_scope),
            acceptance_checks: normalize_acceptance_checks(&kind, spec.acceptance_checks),
            review_metadata: None,
            dependencies: vec![review_task.task_id.clone()],
            execution_generation: review_task.execution_generation,
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: default_task_max_attempts(),
            publish_attempt_count: 0,
            estimated_minutes: Some(normalize_task_estimated_minutes(
                Some(spec.estimated_minutes),
                &kind,
            )),
            status: CoordinationTaskStatus::Blocked,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: now,
            updated_at: now,
        };
        write_json_atomic(&task_path(run_id, &task_id), &task)?;
        run.task_ids.push(task_id.clone());
        generated_ids.push(task_id.clone());
        append_event(
            run_id,
            "task_generated",
            Some(&task_id),
            &format!("Generated review fix task '{}'.", task.title),
        )?;
    }
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix
        && artifact.report.outcome == CoordinationTaskOutcome::Partial
        && generated_ids.is_empty()
    {
        // CI gate: only synthesize a wait task (or any LLM work) if CI actually failed.
        // - Success: stay dormant, no wait, no follow-up review
        // - InProgress: requeue happens via existing requeue_github_branch_fix_poll_completion
        // - Failed: synthesize wait task with tight fix-only plan prompt
        match github_branch_fix_ci_status(&run) {
            Ok(GithubBranchFixCiStatus::Success) => {
                // CI is green — skip everything, stay dormant until next failure
                return Ok(());
            }
            Ok(GithubBranchFixCiStatus::InProgress) => {
                // CI still running — do NOT synthesize a new wait task.
                // The existing requeue logic (claims_completion.rs) will silently
                // requeue any in-progress wait task without consuming attempt budget.
                // No new LLM task needed.
                return Ok(());
            }
            Ok(GithubBranchFixCiStatus::Failed) | Ok(GithubBranchFixCiStatus::Unknown) | Err(_) => {
                // CI failed or unknown — proceed to synthesize wait task, but use
                // a tight fix-only prompt below instead of the full verbose bootstrap prompt
            }
        }
        let wait_spec = github_branch_fix_wait_for_success_spec(&run);
        let wait_index = artifact.report.fix_tasks.len();
        if let Some(task) = tasks.iter_mut().find(|task| {
            task.parent_task_id.as_deref() == Some(review_task.task_id.as_str())
                && task.kind == CoordinationTaskKind::GeneratedReadTask
                && task.title == wait_spec.title
        }) {
            task.assigned_provider =
                resolve_generated_task_provider(&run, wait_spec.provider.as_deref(), wait_index);
            task.task_category =
                default_task_category_for_kind(&CoordinationTaskKind::GeneratedReadTask);
            task.prompt = if uses_lightweight_runtime_mode(&run.workflow_mode) {
                lightweight_generated_task_prompt(
                    &run,
                    &CoordinationTaskKind::GeneratedReadTask,
                    &wait_spec.prompt,
                )
            } else {
                wait_spec.prompt.clone()
            };
            task.cwd = run.workspace_path.clone();
            task.allowed_tools = normalize_allowed_tools(
                &CoordinationTaskKind::GeneratedReadTask,
                wait_spec.allowed_tools.clone(),
            );
            task.read_scope = normalize_read_scope(
                &CoordinationTaskKind::GeneratedReadTask,
                wait_spec.read_scope.clone(),
            );
            task.write_scope = Vec::new();
            task.acceptance_checks = normalize_acceptance_checks(
                &CoordinationTaskKind::GeneratedReadTask,
                wait_spec.acceptance_checks.clone(),
            );
            task.dependencies = vec![review_task.task_id.clone()];
            task.estimated_minutes = Some(normalize_task_estimated_minutes(
                Some(wait_spec.estimated_minutes),
                &CoordinationTaskKind::GeneratedReadTask,
            ));
            task.status = CoordinationTaskStatus::Blocked;
            task.updated_at = now;
            write_json_atomic(&task_path(run_id, &task.task_id), task)?;
            generated_ids.push(task.task_id.clone());
        } else {
            let task_id = Uuid::new_v4().to_string();
            let task = CoordinationTask {
                provider_hint: None,
                required_role: None,
                task_category: default_task_category_for_kind(
                    &CoordinationTaskKind::GeneratedReadTask,
                ),
                task_id: task_id.clone(),
                run_id: run_id.to_string(),
                parent_task_id: Some(review_task.task_id.clone()),
                kind: CoordinationTaskKind::GeneratedReadTask,
                assigned_provider: resolve_generated_task_provider(
                    &run,
                    wait_spec.provider.as_deref(),
                    wait_index,
                ),
                iteration: None,
                title: wait_spec.title,
                prompt: if uses_lightweight_runtime_mode(&run.workflow_mode) {
                    lightweight_generated_task_prompt(
                        &run,
                        &CoordinationTaskKind::GeneratedReadTask,
                        &wait_spec.prompt,
                    )
                } else {
                    wait_spec.prompt
                },
                cwd: run.workspace_path.clone(),
                allowed_tools: normalize_allowed_tools(
                    &CoordinationTaskKind::GeneratedReadTask,
                    wait_spec.allowed_tools,
                ),
                read_scope: normalize_read_scope(
                    &CoordinationTaskKind::GeneratedReadTask,
                    wait_spec.read_scope,
                ),
                write_scope: Vec::new(),
                acceptance_checks: normalize_acceptance_checks(
                    &CoordinationTaskKind::GeneratedReadTask,
                    wait_spec.acceptance_checks,
                ),
                review_metadata: None,
                dependencies: vec![review_task.task_id.clone()],
                execution_generation: review_task.execution_generation,
                dependency_generation: None,
                attempt_count: 0,
                max_attempts: default_task_max_attempts(),
                publish_attempt_count: 0,
                estimated_minutes: Some(normalize_task_estimated_minutes(
                    Some(wait_spec.estimated_minutes),
                    &CoordinationTaskKind::GeneratedReadTask,
                )),
                status: CoordinationTaskStatus::Blocked,
                claimed_by: None,
                artifact_summary: None,
                failure_kind: None,
                recovery_action: None,
                created_at: now,
                updated_at: now,
            };
            write_json_atomic(&task_path(run_id, &task_id), &task)?;
            run.task_ids.push(task_id.clone());
            generated_ids.push(task_id.clone());
            append_event(
                run_id,
                "task_generated",
                Some(&task_id),
                &format!(
                    "Synthesized branch-fix wait task '{}' after partial review.",
                    task.title
                ),
            )?;
        }
    }
    let follow_up_dependencies = if generated_ids.is_empty() {
        vec![review_task.task_id.clone()]
    } else {
        generated_ids.clone()
    };
    let follow_up_prompt = if uses_lightweight_runtime_mode(&run.workflow_mode) {
        lightweight_review_prompt(&run)
    } else {
        review_task.prompt.clone()
    };
    let follow_up_write_scope = if uses_lightweight_runtime_mode(&run.workflow_mode) {
        lightweight_review_write_scope(&run)
    } else {
        review_task.write_scope.clone()
    };
    let follow_up_title = format!("Follow-up review after {}", review_task.title);
    let follow_up_review_id = if let Some(task) = tasks.iter_mut().find(|task| {
        task.parent_task_id.as_deref() == Some(review_task.task_id.as_str())
            && task.kind == CoordinationTaskKind::ReviewArtifacts
            && task.title == follow_up_title
    }) {
        task.assigned_provider = review_task.assigned_provider.clone();
        task.task_category = default_task_category_for_kind(&CoordinationTaskKind::ReviewArtifacts);
        task.prompt = follow_up_prompt.clone();
        task.cwd = review_task.cwd.clone();
        task.allowed_tools = review_task.allowed_tools.clone();
        task.read_scope = review_task.read_scope.clone();
        task.write_scope = follow_up_write_scope.clone();
        task.acceptance_checks = review_task.acceptance_checks.clone();
        task.dependencies = follow_up_dependencies.clone();
        task.estimated_minutes = Some(default_task_estimated_minutes_for_kind(
            &CoordinationTaskKind::ReviewArtifacts,
        ));
        task.status = CoordinationTaskStatus::Blocked;
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        if let Some(metadata) = load_review_task_metadata(run_id, &review_task.task_id) {
            write_review_task_metadata(run_id, &task.task_id, &metadata)?;
        }
        task.task_id.clone()
    } else {
        let follow_up_review_id = Uuid::new_v4().to_string();
        let follow_up_review = CoordinationTask {
            provider_hint: None,
            required_role: None,
            task_category: default_task_category_for_kind(&CoordinationTaskKind::ReviewArtifacts),
            task_id: follow_up_review_id.clone(),
            run_id: run_id.to_string(),
            parent_task_id: Some(review_task.task_id.clone()),
            kind: CoordinationTaskKind::ReviewArtifacts,
            assigned_provider: review_task.assigned_provider.clone(),
            iteration: None,
            title: follow_up_title,
            prompt: follow_up_prompt,
            cwd: review_task.cwd.clone(),
            allowed_tools: review_task.allowed_tools.clone(),
            read_scope: review_task.read_scope.clone(),
            write_scope: follow_up_write_scope,
            acceptance_checks: review_task.acceptance_checks.clone(),
            review_metadata: None,
            dependencies: follow_up_dependencies.clone(),
            execution_generation: review_task.execution_generation,
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: default_task_max_attempts(),
            publish_attempt_count: 0,
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(
                &CoordinationTaskKind::ReviewArtifacts,
            )),
            status: CoordinationTaskStatus::Blocked,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: now,
            updated_at: now,
        };
        write_json_atomic(&task_path(run_id, &follow_up_review_id), &follow_up_review)?;
        if let Some(metadata) = load_review_task_metadata(run_id, &review_task.task_id) {
            write_review_task_metadata(run_id, &follow_up_review_id, &metadata)?;
        }
        run.task_ids.push(follow_up_review_id.clone());
        follow_up_review_id
    };

    for task in tasks.iter_mut() {
        if task.kind != CoordinationTaskKind::ApplyApprovedEdits
            && task.kind != CoordinationTaskKind::MaterializeDesignDoc
        {
            continue;
        }
        if !task
            .dependencies
            .iter()
            .any(|dep| dep == &review_task.task_id)
            && !task
                .dependencies
                .iter()
                .any(|dep| dep == &follow_up_review_id)
        {
            continue;
        }
        task.dependencies = vec![follow_up_review_id.clone()];
        task.status = CoordinationTaskStatus::Blocked;
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
    }

    run.updated_at = Utc::now();
    write_json_atomic(&run_path(run_id), &run)?;
    append_event(
        run_id,
        "review_fix_loop",
        Some(&review_task.task_id),
        &format!(
            "Review materialized {} file(s) directly and queued {} follow-up task(s). Queued a follow-up review before apply.",
            materialized_paths.len(),
            generated_ids.len()
        ),
    )?;
    Ok(())
}

pub fn try_materialize_review_fix_task(
    run_id: &str,
    review_task: &CoordinationTask,
    spec: &GeneratedTaskSpec,
) -> Result<Option<String>> {
    let kind = parse_generated_kind(&spec.kind)?;
    if kind != CoordinationTaskKind::GeneratedWriteTask {
        return Ok(None);
    }

    let missing_targets = spec
        .write_scope
        .iter()
        .map(|scope| normalize_scope(scope))
        .filter(|path| !Path::new(&review_task.cwd).join(path).is_file())
        .collect::<Vec<_>>();
    if missing_targets.len() != 1 {
        return Ok(None);
    }
    let output_path = &missing_targets[0];

    for dependency in &review_task.dependencies {
        let path = artifact_result_path(run_id, dependency);
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let dependency_artifact: TaskArtifact = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let materialized_files = load_artifact_materialized_files(run_id, dependency);
        let Some(content) = materialized_artifact_content_for_path(
            &dependency_artifact,
            &materialized_files,
            output_path,
        ) else {
            continue;
        };

        let candidate = Path::new(&review_task.cwd).join(output_path);
        let parent = candidate
            .parent()
            .context("review fix output path missing parent")?;
        fs::create_dir_all(parent)?;
        fs::write(&candidate, content)
            .with_context(|| format!("failed to write {}", candidate.display()))?;
        append_event(
            run_id,
            "review_fix_materialized",
            Some(&review_task.task_id),
            &format!(
                "Materialized '{}' directly from dependency artifact {}.",
                output_path, dependency
            ),
        )?;
        return Ok(Some(output_path.clone()));
    }

    Ok(None)
}

pub fn rewire_follow_up_tasks(
    run_id: &str,
    source_task_id: &str,
    tasks: &mut [CoordinationTask],
    generated_ids: &[String],
) -> Result<()> {
    for task in tasks.iter_mut() {
        // Skip generated children — they are the newly created tasks (GeneratedReadTask,
        // GeneratedWriteTask) that already have the source as their parent.
        let is_generated_child = task.parent_task_id.as_deref() == Some(source_task_id)
            && is_generated_task_kind(&task.kind);
        if is_generated_child {
            continue;
        }
        // Also skip ReviewArtifacts tasks that appear in generated_ids — these are the
        // review tasks spawned by the plan that should keep their original dependencies.
        if task.kind == CoordinationTaskKind::ReviewArtifacts
            && generated_ids.iter().any(|id| id.as_str() == task.task_id)
        {
            continue;
        }
        if !task.dependencies.iter().any(|dep| dep == source_task_id) {
            continue;
        }
        match task.kind {
            CoordinationTaskKind::PlanFlatTasks => {
                task.dependencies = generated_ids.to_vec();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated downstream planning task dependencies to generated tasks.",
                )?;
            }
            CoordinationTaskKind::ReviewArtifacts => {
                task.dependencies = generated_ids
                    .iter()
                    .filter(|task_id| task_id.as_str() != task.task_id)
                    .cloned()
                    .collect();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated review task dependencies to generated tasks.",
                )?;
            }
            CoordinationTaskKind::MaterializeDesignDoc => {
                task.dependencies = generated_ids.to_vec();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated materialization task dependencies to generated review tasks.",
                )?;
            }
            CoordinationTaskKind::ApplyApprovedEdits => {
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn parse_generated_task_specs(
    body: &str,
    run: Option<&CoordinationRun>,
) -> Result<Vec<GeneratedTaskSpec>> {
    // In lightweight mode (GithubBranchFix, LightweightImplementation), skip the strict
    // typed-envelope contract and accept plain JSON arrays to avoid validation errors when
    // the provider returns loose JSON.
    let is_lightweight = run.map_or(false, |r| uses_lightweight_runtime_mode(&r.workflow_mode));
    if !is_lightweight {
        if lookup_contract(CoordinationTaskKind::PlanFlatTasks).is_some() {
            if let Ok(envelope) = parse_contract_envelope_from_response(body) {
                let payload = plan_tasks_payload_from_envelope(&envelope)?;
                return Ok(payload.generated_tasks);
            }
        }
    }
    let json = extract_first_json_array(body).context("missing JSON task array in plan output")?;
    serde_json::from_str(json).context("invalid generated task JSON")
}

pub fn validate_generated_task_specs(specs: &[GeneratedTaskSpec]) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("generated task list is empty");
    }

    validate_generated_task_dependency_specs(specs)?;

    for spec in specs {
        if spec.title.trim().is_empty() || spec.prompt.trim().is_empty() {
            anyhow::bail!("generated task is missing a title or prompt");
        }
        if spec.provider.is_some() && normalize_provider_name(spec.provider.as_deref()).is_none() {
            anyhow::bail!("generated task '{}' uses an unknown provider", spec.title);
        }
        if !(MIN_TASK_ESTIMATED_MINUTES..=MAX_TASK_ESTIMATED_MINUTES)
            .contains(&spec.estimated_minutes)
        {
            anyhow::bail!(
                "generated task '{}' must set estimated_minutes between {} and {}",
                spec.title,
                MIN_TASK_ESTIMATED_MINUTES,
                MAX_TASK_ESTIMATED_MINUTES
            );
        }

        let kind = parse_generated_kind(&spec.kind)?;
        if kind == CoordinationTaskKind::GeneratedWriteTask {
            if spec.write_scope.is_empty() {
                anyhow::bail!(
                    "generated write task '{}' is missing write_scope",
                    spec.title
                );
            }
            if spec.acceptance_checks.len() < 2 {
                let normalized_checks = normalize_string_list(spec.acceptance_checks.clone());
                if normalized_checks.len() < 2 {
                    anyhow::bail!(
                        "generated write task '{}' is missing concrete acceptance checks",
                        spec.title
                    );
                }
            }
        }

        // Validate scope paths for traversal patterns.
        // Check for `..` path segments which could escape the workspace root,
        // and for absolute paths that could reference files outside the workspace.
        for scope in spec.read_scope.iter().chain(spec.write_scope.iter()) {
            if raw_scope_contains_parent_traversal(scope) {
                anyhow::bail!(
                    "generated task '{}' has scope '{}' containing '..' (parent-directory traversal)",
                    spec.title,
                    scope.trim()
                );
            }
            if raw_scope_is_absolute(scope) {
                anyhow::bail!(
                    "generated task '{}' has absolute scope '{}' which is not allowed",
                    spec.title,
                    scope.trim()
                );
            }

            let normalized = normalize_scope(scope);

            // Reject any path containing a parent-directory segment.
            if normalized.contains("..") {
                anyhow::bail!(
                    "generated task '{}' has normalized scope '{}' containing '..' (parent-directory traversal)",
                    spec.title,
                    normalized
                );
            }
        }
    }

    Ok(())
}
