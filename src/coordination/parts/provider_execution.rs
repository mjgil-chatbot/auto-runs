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
#[path = "provider_execution_retries.rs"]
mod provider_execution_retries;
pub use provider_execution_retries::{
    requeue_task_for_provider_cooldown, requeue_task_for_provider_timeout,
};

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
    }
}

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

pub fn execute_provider_task(
    task: &CoordinationTask,
    provider: &str,
    instance_pid: u32,
    stop_signal: Arc<AtomicBool>,
) -> Result<()> {
    let run = load_run(&task.run_id)?;
    let provider_name = task.assigned_provider.as_deref().unwrap_or(provider);
    let input_decision = check_input_rail(&run, task);
    if !matches!(input_decision, RailDecision::Allow) {
        let _ = persist_rail_decision(
            &run,
            task,
            provider_name,
            &RailStage::Input,
            &input_decision,
        );
        match &input_decision {
            RailDecision::Reject { reason } => {
                anyhow::bail!("input rail rejected task: {}", reason)
            }
            RailDecision::Interrupt { kind, .. } => {
                anyhow::bail!("input rail interrupted task: {:?}", kind)
            }
            RailDecision::Reask { message } => {
                anyhow::bail!("input rail requested reask: {}", message)
            }
            RailDecision::Rewrite { .. } => anyhow::bail!("input rail returned unexpected rewrite"),
            RailDecision::Allow => unreachable!(),
        }
    }
    heartbeat_claim(&task.run_id, &task.task_id, instance_pid)?;
    ensure_task_workspace_exists(&run, task, Some(provider_name))?;
    let _runsc_env_guard = AutonomousRunscEnvGuard::activate(&run)?;
    append_runsc_ready_event(&run, task, provider_name)?;
    let execution_task = task_for_provider_execution(task, &run);
    let contract = lookup_contract_for_task(task);
    let provider_model = resolve_autonomous_provider(provider_name)?;
    let mut overrides = autonomous_provider_overrides(provider_model);
    if let Some(contract) = contract {
        let mode = apply_structured_output_overrides(provider_name, contract, &mut overrides)?;
        debug!(
            run_id = %task.run_id,
            task_id = %task.task_id,
            provider = provider_name,
            ?mode,
            "selected structured output mode for provider task"
        );
    }
    let _runsc_cwd_guard = if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        Some(RunscCwdGuard::set(Path::new(&execution_task.cwd))?)
    } else {
        None
    };
    let mut query = build_provider_query(task)?;
    let dialog_decision = check_dialog_rail(task, &query);
    if !matches!(dialog_decision, RailDecision::Allow) {
        let _ = persist_rail_decision(
            &run,
            task,
            provider_name,
            &RailStage::Dialog,
            &dialog_decision,
        );
        match dialog_decision {
            RailDecision::Allow => {}
            RailDecision::Rewrite { replacement } => query = replacement,
            RailDecision::Reject { reason } => {
                anyhow::bail!("dialog rail rejected task: {}", reason)
            }
            RailDecision::Reask { message } => {
                anyhow::bail!("dialog rail requested reask: {}", message)
            }
            RailDecision::Interrupt { kind, .. } => {
                anyhow::bail!("dialog rail interrupted task: {:?}", kind)
            }
        }
    }
    let memory_context = match memory::retrieve_memory_context(&task.cwd, &query) {
        Ok(ctx) => ctx,
        Err(err) => {
            warn!(
                task_id = %task.task_id,
                error = %err,
                "memory context retrieval failed; proceeding without memory"
            );
            None
        }
    };
    let write_scope_snapshot = snapshot_declared_write_scope(task);
    let _ = record_task_execution_started(&task.run_id, &task.task_id, instance_pid);
    let session = Some(make_task_session(&execution_task));
    let heartbeat_stop = Arc::new(AtomicBool::new(false));
    let heartbeat_lost_ownership = Arc::new(AtomicBool::new(false));
    let heartbeat_stop_thread = Arc::clone(&heartbeat_stop);
    let heartbeat_lost_ownership_thread = Arc::clone(&heartbeat_lost_ownership);
    let run_id = task.run_id.clone();
    let task_id = task.task_id.clone();
    let provider_name_owned = provider_name.to_string();
    let heartbeat_task = task.clone();
    let heartbeat_workspace_root = execution_task.cwd.clone();
    let heartbeat_handle = thread::spawn(move || {
        fn lcg_rand(seed: &mut u64) -> u32 {
            const RAND_MULT: u64 = 1103515245;
            const RAND_INC: u64 = 12345;
            const RAND_MOD: u64 = 1 << 63;
            *seed = seed.wrapping_mul(RAND_MULT).wrapping_add(RAND_INC) % RAND_MOD;
            (*seed >> 33) as u32
        }
        let mut seed: u64 = task_id
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_add(b as u64).wrapping_mul(31));
        let mut heartbeat_failures: u32 = 0;
        let mut workspace_activity =
            WorkspaceActivityObserver::new(PathBuf::from(heartbeat_workspace_root));
        while !heartbeat_stop_thread.load(Ordering::Acquire) {
            if let Err(err) = heartbeat_claim(&run_id, &task_id, instance_pid) {
                let is_ownership_error = err.downcast_ref::<ClaimError>().is_some();
                if is_ownership_error {
                    heartbeat_lost_ownership_thread.store(true, Ordering::Release);
                    break;
                }
                heartbeat_failures += 1;
                if heartbeat_failures <= 3 {
                    warn!(
                        task_id = %task_id,
                        attempt = heartbeat_failures,
                        error = %err,
                        "heartbeat_claim failed"
                    );
                }
            } else {
                heartbeat_failures = 0;
                if let Err(err) = workspace_activity.observe_and_maybe_emit_summary(
                    &run_id,
                    &heartbeat_task,
                    &provider_name_owned,
                ) {
                    warn!(
                        run_id = %run_id,
                        task_id = %task_id,
                        error = %err,
                        "workspace activity scan failed"
                    );
                }
            }
            let jitter = lcg_rand(&mut seed) % 1000;
            let sleep_duration = Duration::from_secs(TASK_HEARTBEAT_SECS)
                + Duration::from_millis(jitter as u64)
                - Duration::from_millis(500);
            thread::sleep(sleep_duration);
        }
    });

    let (tx, rx) = mpsc::channel();
    let provider_timeout = provider_task_timeout(provider_name, task);
    let provider_stop_signal = provider_timeout
        .map(|_| Arc::new(AtomicBool::new(false)))
        .unwrap_or_else(|| Arc::clone(&stop_signal));
    let provider_timed_out = Arc::new(AtomicBool::new(false));
    let provider_timeout_stop = Arc::new(AtomicBool::new(false));
    let timeout_handle = provider_timeout.map(|timeout| {
        let stop_signal = Arc::clone(&stop_signal);
        let provider_stop_signal = Arc::clone(&provider_stop_signal);
        let provider_timed_out = Arc::clone(&provider_timed_out);
        let provider_timeout_stop = Arc::clone(&provider_timeout_stop);
        thread::spawn(move || {
            let started_at = std::time::Instant::now();
            while !provider_timeout_stop.load(Ordering::Acquire) {
                if stop_signal.load(Ordering::Acquire) {
                    provider_stop_signal.store(true, Ordering::Release);
                    return;
                }
                if started_at.elapsed() >= timeout {
                    provider_timed_out.store(true, Ordering::Release);
                    provider_stop_signal.store(true, Ordering::Release);
                    return;
                }
                thread::sleep(Duration::from_millis(100));
            }
        })
    });
    let result = call_provider_cli_stream_with_overrides(
        provider_model,
        &query,
        Some(TASK_SYSTEM_PROMPT),
        memory_context.as_ref(),
        session,
        Some(&overrides),
        tx,
        Arc::clone(&provider_stop_signal),
    );

    let mut response = collect_stream_response(rx);
    if stop_signal.load(Ordering::Acquire) {
        provider_timeout_stop.store(true, Ordering::Release);
        if let Some(handle) = timeout_handle {
            let _ = handle.join();
        }
        heartbeat_stop.store(true, Ordering::Release);
        let _ = heartbeat_handle.join();
        return Ok(());
    }

    if let Ok(()) = result.as_ref() {
        if let Some(contract) = contract {
            response = run_validation_reasks(
                task,
                provider_name,
                &query,
                response,
                contract,
                provider_model,
                &overrides,
                Arc::clone(&provider_stop_signal),
                Arc::clone(&stop_signal),
            )?;
        }
    }

    provider_timeout_stop.store(true, Ordering::Release);
    if let Some(handle) = timeout_handle {
        let _ = handle.join();
    }

    heartbeat_stop.store(true, Ordering::Release);
    let _ = heartbeat_handle.join();

    if heartbeat_lost_ownership.load(Ordering::Acquire) {
        return Ok(());
    }

    if stop_signal.load(Ordering::Acquire) {
        return Ok(());
    }
    if provider_timed_out.load(Ordering::Acquire) {
        let timeout = provider_timeout.expect("provider timeout must exist when timer fires");
        warn!(
            run_id = %task.run_id,
            task_id = %task.task_id,
            provider = provider_name,
            timeout_secs = timeout.as_secs(),
            "provider task execution timed out"
        );
        anyhow::bail!(provider_timeout_failure(provider_name, timeout));
    }
    debug!(
        run_id = %task.run_id,
        task_id = %task.task_id,
        provider = provider_name,
        title = %task.title,
        response_len = response.len(),
        response = %response,
        "collected final provider task response"
    );
    if let Some(provider_failure) = extract_provider_command_failure(&response) {
        anyhow::bail!(provider_failure.to_string());
    }
    if let Some(contract) = contract {
        if let Ok(normalized_response) =
            normalize_contract_response_for_task(contract, &response, task)
        {
            response = normalized_response;
        }
    }
    match result {
        Ok(()) => {
            let artifact = build_provider_task_artifact(task, &response)?;
            let materialized_files =
                capture_materialized_files_from_snapshot(task, &write_scope_snapshot);
            complete_task_with_materialized_files(
                &task.run_id,
                &task.task_id,
                instance_pid,
                artifact,
                materialized_files,
            )
        }
        Err(err) => Err(err),
    }
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

pub fn run_provider_task_for_claim(
    run_id: &str,
    task_id: &str,
    provider: &str,
    instance_pid: u32,
    stop_signal: Arc<AtomicBool>,
) -> Result<()> {
    let claim = load_claim(run_id, task_id)?;
    if claim.instance_pid != instance_pid {
        anyhow::bail!(
            "task {} for run {} is owned by PID {}, not {}",
            task_id,
            run_id,
            claim.instance_pid,
            instance_pid
        );
    }

    let task = load_task(run_id, task_id)?;
    match execute_provider_task(&task, provider, instance_pid, Arc::clone(&stop_signal)) {
        Ok(()) => Ok(()),
        Err(err) => {
            if stop_signal.load(Ordering::Acquire) {
                return Ok(());
            }
            warn!(
                run_id = %run_id,
                task_id = %task_id,
                provider = %provider,
                error = %err,
                "provider task failed before structured artifact completion"
            );
            if requeue_task_for_provider_cooldown(
                run_id,
                task_id,
                instance_pid,
                provider,
                &err.to_string(),
            )? {
                warn!(
                    run_id = %run_id,
                    task_id = %task_id,
                    provider = %provider,
                    "provider task failure was converted into a cooldown retry"
                );
                return Ok(());
            }
            if requeue_task_for_provider_timeout(
                run_id,
                task_id,
                instance_pid,
                provider,
                &err.to_string(),
            )? {
                warn!(
                    run_id = %run_id,
                    task_id = %task_id,
                    provider = %provider,
                    "provider task timeout was converted into a polling retry"
                );
                return Ok(());
            }
            if let Err(fail_err) = fail_task(run_id, task_id, instance_pid, &err.to_string()) {
                warn!(
                    run_id = %run_id,
                    task_id = %task_id,
                    error = %fail_err,
                    "failed to mark task as failed after provider error"
                );
            }
            Err(err)
        }
    }
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
