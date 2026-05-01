use super::*;
#[allow(unused_imports)]
use crate::model::ModelProvider;
#[allow(unused_imports)]
use crate::provider_bridge::call_provider_cli_stream_with_overrides;
#[allow(unused_imports)]
use crate::AutonomousExecutionEnvironment;
#[allow(unused_imports)]
use crate::AutonomousWorkflowMode;
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
use std::env;
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
pub fn should_force_skip_git_repo_check(run: &CoordinationRun, task: &CoordinationTask) -> bool {
    matches!(run.workflow_mode, AutonomousWorkflowMode::PerfHillClimber)
        && matches!(
            task.kind,
            CoordinationTaskKind::PerfBaseline
                | CoordinationTaskKind::PerfParse
                | CoordinationTaskKind::PerfBench
                | CoordinationTaskKind::PerfEvaluate
                | CoordinationTaskKind::PerfApply
        )
}
struct RunscCwdGuard {
    original_cwd: PathBuf,
}
impl RunscCwdGuard {
    fn set(new_cwd: &Path) -> Result<Self> {
        let original_cwd = env::current_dir()?;
        env::set_current_dir(new_cwd)?;
        Ok(Self { original_cwd })
    }}
impl Drop for RunscCwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original_cwd);
    }
}
fn task_for_provider_execution(task: &CoordinationTask, run: &CoordinationRun) -> CoordinationTask {
    let mut adjusted = task.clone();
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        adjusted.cwd = autonomous_runsc_workspace_root(&run.run_id)
            .to_string_lossy()
            .into_owned();
    }
    adjusted
}
pub(crate) fn append_runsc_ready_event(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider_name: &str,
) -> Result<()> {
    if run.execution_environment != AutonomousExecutionEnvironment::Runsc {
        return Ok(());
    }

    let includes = effective_runsc_workspace_includes(run, Some(task));
    let include_summary = if includes.is_empty() {
        "Explicit includes: none".to_string()
    } else {
        format!("Explicit includes: {}", includes.join(", "))
    };
    append_event_with_provider(
        &run.run_id,
        "task_runsc_ready",
        Some(&task.task_id),
        Some(provider_name),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Runsc started correctly for {} via {}. Shared workspace root: {}. {}.",
            task.title,
            provider_name,
            autonomous_runsc_workspace_root(&run.run_id).display(),
            include_summary
        ),
    )
}
fn run_validation_reasks(
    task: &CoordinationTask,
    provider_name: &str,
    base_query: &str,
    initial_response: String,
    contract: &TaskContract,
    provider: crate::model::ModelProvider,
    overrides: &crate::provider_bridge::ProviderRequestOverrides,
    provider_stop_signal: Arc<AtomicBool>,
    user_stop_signal: Arc<AtomicBool>,
) -> Result<String> {
    let max_reasks = contract.max_validation_reasks as u32;
    if max_reasks == 0 {
        return Ok(initial_response);
    }

    let mut current = initial_response;
    let mut attempt: u32 = 0;
    while attempt < max_reasks {
        let errors = collect_typed_validation_errors(task, contract, &current);
        if errors.is_empty() {
            return Ok(current);
        }

        attempt += 1;
        let reask = build_reask_from_errors(contract, &errors);
        warn!(
            run_id = %task.run_id,
            task_id = %task.task_id,
            provider = %provider_name,
            attempt = attempt,
            errors = ?errors,
            "contract validation errors for reask"
        );
        if let Err(err) =
            append_reask_event(&task.run_id, &task.task_id, provider_name, &reask, attempt)
        {
            warn!(
                run_id = %task.run_id,
                task_id = %task.task_id,
                error = %err,
                "failed to append reask event"
            );
        }

        let reask_query = format!("{}\n\n{}", base_query, reask.message);
        let session = Some(make_task_session(task));
        let (tx2, rx2) = mpsc::channel();
        let call_result = call_provider_cli_stream_with_overrides(
            provider,
            &reask_query,
            Some(TASK_SYSTEM_PROMPT),
            None,
            session,
            Some(overrides),
            tx2,
            Arc::clone(&provider_stop_signal),
        );
        let next_response = collect_stream_response(rx2);
        if user_stop_signal.load(Ordering::Acquire) {
            return Ok(current);
        }
        if call_result.is_err() {
            return Ok(current);
        }
        current = next_response;
    }

    let final_errors = collect_typed_validation_errors(task, contract, &current);
    if !final_errors.is_empty() {
        warn!(
            run_id = %task.run_id,
            task_id = %task.task_id,
            attempts = attempt,
            "reask attempts exhausted; recording terminal failure report"
        );
        let report = terminal_reask_failure_report(&current, attempt);
        return Ok(serde_json::to_string(&report)?);
    }
    Ok(current)
}
fn collect_typed_validation_errors(
    task: &CoordinationTask,
    contract: &TaskContract,
    response: &str,
) -> Vec<String> {
    validate_contract_response_for_task(contract, response, task)
}
fn append_reask_event(
    run_id: &str,
    task_id: &str,
    provider_name: &str,
    reask: &ValidationReask,
    attempt: u32,
) -> Result<()> {
    let event = reask_event(task_id, Some(provider_name), reask, attempt);
    append_event_with_provider(
        run_id,
        &event.event_type,
        event.task_id.as_deref(),
        event.provider.as_deref(),
        event.category,
        event.fallback_from.as_deref(),
        &event.message,
    )
}
pub fn build_provider_query(task: &CoordinationTask) -> Result<String> {
    let response_contract = provider_response_contract(task);
    let dependency_artifacts = load_dependency_artifacts_for_task(task)?;
    if dependency_artifacts.is_empty() && response_contract.is_empty() {
        return Ok(task.prompt.clone());
    }

    let mut sections = vec![task.prompt.clone()];
    if !dependency_artifacts.is_empty() {
        sections.push(format!(
            "Dependency artifacts:\n{}",
            dependency_artifacts.join("\n\n---\n\n")
        ));
    }
    if !response_contract.is_empty() {
        sections.push(response_contract);
    }
    Ok(sections.join("\n\n"))
}
pub use super::provider_contracts::{
    provider_response_contract, provider_visible_workspace_root, task_report_validator_hint,
};
#[cfg(test)]
#[test]
fn task_for_provider_execution_rewrites_cwd_into_shared_runsc_workspace() {
    let now = Utc::now();
    let run = CoordinationRun {
        run_id: "runsc-run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: "/tmp/host-workspace".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Runsc,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "goal".to_string(),
        workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
        design_doc_path: "docs/autonomous/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: now,
        updated_at: now,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: run.run_id.clone(),
        title: "Test".to_string(),
        prompt: "prompt".to_string(),
        cwd: run.workspace_path.clone(),
        status: CoordinationTaskStatus::Queued,
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };

    let adjusted = task_for_provider_execution(&task, &run);

    assert_eq!(
        adjusted.cwd,
        autonomous_runsc_workspace_root(&run.run_id)
            .to_string_lossy()
            .into_owned()
    );
}
#[cfg(test)]
#[test]
fn task_for_provider_execution_keeps_host_cwd_unchanged() {
    let now = Utc::now();
    let run = CoordinationRun {
        run_id: "host-run".to_string(),
        root_chat_id: "chat".to_string(),
        workspace_path: "/tmp/host-workspace".to_string(),
        execution_environment: AutonomousExecutionEnvironment::Host,
        runsc_workspace_includes: Vec::new(),
        provider: "codex".to_string(),
        goal: "goal".to_string(),
        workflow_mode: AutonomousWorkflowMode::PerfHillClimber,
        design_doc_path: "docs/autonomous/design.md".to_string(),
        completion_criteria: String::new(),
        max_iterations: 1,
        status: CoordinationRunStatus::Active,
        created_at: now,
        updated_at: now,
        created_by_instance: None,
        max_agents: 1,
        allow_write_tasks: true,
        require_review_before_apply: false,
        auto_retry_failed_tasks: true,
        notes: String::new(),
        review_scope: None,
        review_dimensions: None,
        review_fix_mode: None,
        council_turn_policy: None,
        council_max_turns: None,
        provider_cooldowns: BTreeMap::new(),
        task_ids: Vec::new(),
        processed_mutation_batch_ids: BTreeSet::new(),
    };
    let task = CoordinationTask {
        task_id: "task".to_string(),
        run_id: run.run_id.clone(),
        title: "Test".to_string(),
        prompt: "prompt".to_string(),
        cwd: run.workspace_path.clone(),
        status: CoordinationTaskStatus::Queued,
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };

    let adjusted = task_for_provider_execution(&task, &run);

    assert_eq!(adjusted.cwd, task.cwd);
}
