use fs2::FileExt;
pub fn requeue_failed_tasks_for_retry(
    run_id: &str,
    run: &CoordinationRun,
    tasks: &mut [CoordinationTask],
) -> Result<bool> {
    let mut changed = false;
    for task in tasks.iter_mut() {
        if task.status != CoordinationTaskStatus::Failed
            || !should_auto_retry_task(run, task, task.artifact_summary.as_deref())
        {
            continue;
        }

        let previous_attempt = task.attempt_count.max(1);
        task.status = CoordinationTaskStatus::Queued;
        task.claimed_by = None;
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        append_event(
            run_id,
            "task_retry_scheduled",
            Some(&task.task_id),
            &format!(
                "Retry scheduled: {} after attempt {}/{}. {}",
                task.title,
                previous_attempt,
                task.max_attempts,
                retry_reason_from_summary(task.artifact_summary.as_deref())
            ),
        )?;
        changed = true;
    }
    Ok(changed)
}
pub fn should_auto_retry_task(
    run: &CoordinationRun,
    task: &CoordinationTask,
    failure_message: Option<&str>,
) -> bool {
    let failure_message = failure_message.unwrap_or("");
    let failure_kind = task
        .failure_kind
        .unwrap_or_else(|| infer_failure_kind(failure_message));
    run.auto_retry_failed_tasks
        && task.attempt_count < task.max_attempts.max(1)
        && !is_pre_execution_rail_failure(failure_message)
        && failure_kind != TaskFailureKind::BlockedWrite
        && failure_kind != TaskFailureKind::ProviderUnavailable
        && failure_kind != TaskFailureKind::Publish
}
pub fn blocked_write_failure_requires_manual_restart(task: &CoordinationTask) -> bool {
    task_mutates_workspace(task)
        && task
            .artifact_summary
            .as_deref()
            .map(|summary| summary.contains("[blocked]"))
            .unwrap_or(false)
}
pub fn provider_unavailable_failure_requires_manual_restart(task: &CoordinationTask) -> bool {
    task.artifact_summary
        .as_deref()
        .map(|summary| summary.contains("[provider_unavailable]"))
        .unwrap_or(false)
}
pub fn git_publish_failure_requires_manual_restart(
    task: &CoordinationTask,
    failure_message: Option<&str>,
) -> bool {
    if let Some(msg) = failure_message {
        if msg.contains("[git_publish_failed]") {
            return true;
        }
    }
    task.artifact_summary
        .as_deref()
        .map(|summary| summary.contains("[git_publish_failed]"))
        .unwrap_or(false)
}
pub fn retry_reason_from_summary(summary: Option<&str>) -> String {
    summary
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .unwrap_or("Task failed without a recorded summary.")
        .to_string()
}
pub fn prune_expired_provider_cooldown_map(
    cooldowns: &mut BTreeMap<String, ProviderCooldown>,
    now: DateTime<Utc>,
) -> bool {
    let before = cooldowns.len();
    cooldowns.retain(|_, cooldown| cooldown.unavailable_until > now);
    cooldowns.len() != before
}
pub fn prune_expired_provider_cooldowns(run: &mut CoordinationRun, now: DateTime<Utc>) -> bool {
    prune_expired_provider_cooldown_map(&mut run.provider_cooldowns, now)
}
pub fn refresh_run_provider_cooldowns(
    run: &mut CoordinationRun,
    tasks: &[CoordinationTask],
    now: DateTime<Utc>,
) -> Result<bool> {
    let mut global_provider_cooldowns = load_global_provider_cooldowns()?;
    let global_changed = prune_expired_provider_cooldown_map(&mut global_provider_cooldowns, now);
    let mut run_changed = prune_expired_provider_cooldowns(run, now)
        || merge_provider_cooldowns(&mut run.provider_cooldowns, &global_provider_cooldowns);
    let mut global_changed = global_changed;

    for task in tasks {
        if !matches!(
            task.status,
            CoordinationTaskStatus::Failed | CoordinationTaskStatus::Canceled
        ) || !is_provider_backed(task)
        {
            continue;
        }

        let provider_name = task
            .assigned_provider
            .as_deref()
            .unwrap_or(run.provider.as_str());
        let Some(summary) = task.artifact_summary.as_deref() else {
            continue;
        };
        let Some(cooldown) = extract_provider_cooldown(provider_name, summary, now) else {
            continue;
        };
        let normalized_provider = normalize_provider_name(Some(provider_name))
            .unwrap_or_else(|| provider_name.to_string());
        run_changed |=
            upsert_provider_cooldown(&mut run.provider_cooldowns, &normalized_provider, &cooldown);
        global_changed |= upsert_provider_cooldown(
            &mut global_provider_cooldowns,
            &normalized_provider,
            &cooldown,
        );
    }

    if global_changed {
        write_global_provider_cooldowns(&global_provider_cooldowns)?;
    }
    Ok(run_changed)
}
pub fn upsert_provider_cooldown(
    cooldowns: &mut BTreeMap<String, ProviderCooldown>,
    provider: &str,
    cooldown: &ProviderCooldown,
) -> bool {
    match cooldowns.get(provider) {
        Some(existing) if existing.unavailable_until >= cooldown.unavailable_until => false,
        _ => {
            cooldowns.insert(provider.to_string(), cooldown.clone());
            true
        }
    }
}
pub fn merge_provider_cooldowns(
    destination: &mut BTreeMap<String, ProviderCooldown>,
    source: &BTreeMap<String, ProviderCooldown>,
) -> bool {
    let mut changed = false;
    for (provider, cooldown) in source {
        changed |= upsert_provider_cooldown(destination, provider, cooldown);
    }
    changed
}
pub fn is_provider_on_cooldown<'a>(
    run: &'a CoordinationRun,
    provider: &str,
    now: DateTime<Utc>,
) -> Option<&'a ProviderCooldown> {
    let normalized = normalize_provider_name(Some(provider))?;
    let cooldown = run.provider_cooldowns.get(normalized.as_str())?;
    (cooldown.unavailable_until > now).then_some(cooldown)
}
/// Validate run input at the API boundary to catch configuration errors early.
pub fn validate_run_input(input: &UpsertRunInput) -> Result<()> {
    if input.goal.trim().is_empty() {
        anyhow::bail!("goal is required and cannot be empty");
    }
    if input.goal.len() > 10_000 {
        anyhow::bail!("goal exceeds maximum length of 10,000 characters");
    }
    if input.workspace_path.trim().is_empty() {
        anyhow::bail!("workspace_path is required and cannot be empty");
    }
    if input.provider.trim().is_empty() {
        anyhow::bail!("provider is required and cannot be empty");
    }
    // Validate provider is a known provider
    let provider_lower = input.provider.trim().to_lowercase();
    let known_providers = [
        "claude", "codex", "gemini", "qwen", "minimax", "glm", "nemotron",
    ];
    if !known_providers.contains(&provider_lower.as_str()) {
        anyhow::bail!(
            "unknown provider '{}'. Known providers: {}",
            input.provider,
            known_providers.join(", ")
        );
    }
    if input.max_iterations == 0 {
        anyhow::bail!("max_iterations must be at least 1");
    }
    if input.max_iterations > 100 {
        anyhow::bail!("max_iterations cannot exceed 100");
    }
    if input.max_agents == 0 {
        anyhow::bail!("max_agents must be at least 1");
    }
    if input.max_agents > 32 {
        anyhow::bail!("max_agents cannot exceed 32");
    }
    if input.council_max_turns.is_some_and(|v| v > 200) {
        anyhow::bail!("council_max_turns cannot exceed 200");
    }
    if input.workflow_mode == AutonomousWorkflowMode::LightweightImplementation {
        validate_lightweight_notes(&input.notes, &input.design_doc_path)?;
    }
    if input.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        validate_github_branch_fix_notes(&input.notes)?;
    }
    validate_github_branch_fix_workspace_input(input)?;
    validate_perf_workspace_input(input)?;
    Ok(())
}
pub fn stale_claim_recovery_status(task: &CoordinationTask) -> CoordinationTaskStatus {
    let _ = task;
    CoordinationTaskStatus::Queued
}
const AUTO_FIX_FAILED_RUN_RESTART_LIMIT_NOTE_KEY: &str = "auto_fix_failed_run_restarts";
const AUTO_FIX_FAILED_RUN_RESTARTS_USED_NOTE_KEY: &str = "auto_fix_failed_run_restarts_used";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartMode {
    Manual,
    AutoSelfHeal,
}
pub fn auto_fix_failed_run_restart_limit(run: &CoordinationRun) -> u32 {
    if let Some(limit) = note_setting_value(&run.notes, AUTO_FIX_FAILED_RUN_RESTART_LIMIT_NOTE_KEY)
        .and_then(|value| value.parse::<u32>().ok())
    {
        return limit;
    }

    if run.auto_retry_failed_tasks
        && (uses_lightweight_runtime_mode(&run.workflow_mode)
            || run.workflow_mode == AutonomousWorkflowMode::PerfHillClimber)
    {
        1
    } else {
        0
    }
}
pub fn auto_fix_failed_run_restarts_used(run: &CoordinationRun) -> u32 {
    note_setting_value(&run.notes, AUTO_FIX_FAILED_RUN_RESTARTS_USED_NOTE_KEY)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0)
}
pub fn repair_perf_workspace_for_manual_restart(
    run_id: &str,
    run: &mut CoordinationRun,
    tasks: &mut [CoordinationTask],
    now: DateTime<Utc>,
) -> Result<bool> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(false);
    }

    let mut changed = ensure_perf_workspace_ready(run)?;
    let workspace = perf_workspace_path_for_run(run);
    for task in tasks.iter_mut() {
        if task.cwd == workspace {
            continue;
        }
        task.cwd = workspace.clone();
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        changed = true;
    }

    Ok(changed)
}
pub fn repair_lightweight_workspace_for_manual_restart(
    run_id: &str,
    run: &mut CoordinationRun,
    tasks: &mut [CoordinationTask],
    now: DateTime<Utc>,
) -> Result<bool> {
    if !uses_lightweight_runtime_mode(&run.workflow_mode)
        || run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix
    {
        return Ok(false);
    }

    let workspace = run.workspace_path.clone();
    let mut changed = false;
    for task in tasks.iter_mut() {
        if task.cwd == workspace {
            continue;
        }
        task.cwd = workspace.clone();
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        changed = true;
    }

    if let Some(task) = tasks.first() {
        let _ = ensure_task_workspace_exists(run, task, None)?;
    }

    Ok(changed)
}
fn existing_post_failure_repair_task_id(tasks: &[CoordinationTask]) -> Option<String> {
    tasks.iter().find_map(|task| {
        (task.title == "Repair repository after failed run"
            && task.kind == CoordinationTaskKind::GeneratedWriteTask
            && task.parent_task_id.is_none()
            && matches!(
                task.status,
                CoordinationTaskStatus::Queued
                    | CoordinationTaskStatus::Blocked
                    | CoordinationTaskStatus::Running
                    | CoordinationTaskStatus::Claiming
                    | CoordinationTaskStatus::ArtifactReady
            ))
        .then(|| task.task_id.clone())
    })
}
pub(crate) struct FailedRunRestartLock(fs::File);
impl Drop for FailedRunRestartLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn failed_run_restart_lock_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("restart-failed.lock")
}
pub(crate) fn acquire_failed_run_restart_lock(run_id: &str) -> Result<FailedRunRestartLock> {
    let path = failed_run_restart_lock_path(run_id);
    let parent = path
        .parent()
        .context("failed run restart lock path missing parent")?;
    fs::create_dir_all(parent)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    file.lock_exclusive()
        .with_context(|| format!("failed to lock {}", path.display()))?;
    Ok(FailedRunRestartLock(file))
}
pub fn repair_github_branch_fix_workspace_for_manual_restart(
    run_id: &str,
    run: &mut CoordinationRun,
    tasks: &mut [CoordinationTask],
    now: DateTime<Utc>,
) -> Result<bool> {
    if run.workflow_mode != AutonomousWorkflowMode::GithubBranchFix {
        return Ok(false);
    }

    let mut changed = ensure_github_branch_fix_workspace_ready(run)?;
    let workspace = run.workspace_path.clone();
    for task in tasks.iter_mut() {
        if task.cwd == workspace {
            continue;
        }
        task.cwd = workspace.clone();
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        changed = true;
    }

    Ok(changed)
}
pub fn persist_restarted_task(
    run_id: &str,
    task: &CoordinationTask,
    archived_generation: u32,
) -> Result<()> {
    // Clean up claim and active artifact files from any prior attempt. When a
    // task is moved to a new execution generation, archive the active artifact
    // payloads first so restart history remains inspectable without polluting
    // the new generation's dependency reads.
    archive_active_task_artifacts(run_id, &task.task_id, archived_generation)?;
    let cleanup = |p: PathBuf| {
        if p.is_file() {
            if let Err(err) = fs::remove_file(&p) {
                debug!(
                    path = %p.display(),
                    error = %err,
                    "failed to remove stale artifact file during restart"
                );
            }
        }
    };
    cleanup(claim_path(run_id, &task.task_id));
    cleanup(artifact_result_path(run_id, &task.task_id));
    cleanup(artifact_notes_path(run_id, &task.task_id));
    cleanup(artifact_materialized_files_path(run_id, &task.task_id));
    write_json_atomic(&task_path(run_id, &task.task_id), task)
}
fn archive_active_task_artifacts(run_id: &str, task_id: &str, generation: u32) -> Result<()> {
    let files = [
        (
            artifact_result_path(run_id, task_id),
            archived_artifact_result_path(run_id, task_id, generation),
        ),
        (
            artifact_notes_path(run_id, task_id),
            archived_artifact_notes_path(run_id, task_id, generation),
        ),
        (
            artifact_materialized_files_path(run_id, task_id),
            archived_artifact_materialized_files_path(run_id, task_id, generation),
        ),
        (
            artifact_contract_meta_path(run_id, task_id),
            artifact_history_dir(run_id, task_id, generation).join("contract.json"),
        ),
    ];
    for (active_path, archived_path) in files {
        if !active_path.is_file() || archived_path.is_file() {
            continue;
        }
        if let Some(parent) = archived_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&active_path, &archived_path)?;
    }
    Ok(())
}
pub fn reset_downstream_tasks_for_restart(
    run_id: &str,
    tasks: &mut [CoordinationTask],
    restarted_task_ids: &BTreeSet<String>,
    new_generation: u32,
    now: DateTime<Utc>,
) -> Result<usize> {
    let mut dependency_graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut dependents_graph: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for task in tasks.iter() {
        dependency_graph.insert(task.task_id.clone(), task.dependencies.clone());
        for dependency in &task.dependencies {
            dependents_graph
                .entry(dependency.clone())
                .or_default()
                .push(task.task_id.clone());
        }
    }

    let mut upstream = BTreeSet::new();
    let mut frontier: Vec<String> = restarted_task_ids.iter().cloned().collect();
    while let Some(task_id) = frontier.pop() {
        let Some(dependencies) = dependency_graph.get(&task_id) else {
            continue;
        };
        for dependency in dependencies {
            if upstream.insert(dependency.clone()) {
                frontier.push(dependency.clone());
            }
        }
    }

    for task in tasks.iter_mut() {
        if restarted_task_ids.contains(&task.task_id) || !upstream.contains(&task.task_id) {
            continue;
        }
        task.execution_generation = new_generation;
        task.dependency_generation = Some(new_generation);
        task.recovery_action = None;
        task.updated_at = now;
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
        append_event(
            run_id,
            "task_rebased_for_restart",
            Some(&task.task_id),
            &format!(
                "Rebased completed dependency onto restart generation: {}",
                task.title
            ),
        )?;
    }

    let mut invalidated = restarted_task_ids.clone();
    frontier = restarted_task_ids.iter().cloned().collect();
    while let Some(task_id) = frontier.pop() {
        let Some(dependents) = dependents_graph.get(&task_id) else {
            continue;
        };
        for dependent in dependents {
            if invalidated.insert(dependent.clone()) {
                frontier.push(dependent.clone());
            }
        }
    }
    let mut reset_count = 0usize;

    for task in tasks.iter_mut() {
        if restarted_task_ids.contains(&task.task_id) || !invalidated.contains(&task.task_id) {
            continue;
        }
        let archived_generation = task.execution_generation;
        task.execution_generation = new_generation;
        task.dependency_generation = Some(new_generation);
        task.status = CoordinationTaskStatus::Blocked;
        task.claimed_by = None;
        task.artifact_summary = None;
        task.failure_kind = None;
        task.recovery_action = None;
        task.updated_at = now;
        persist_restarted_task(run_id, task, archived_generation)?;
        append_event(
            run_id,
            "task_reset_for_dependency_restart",
            Some(&task.task_id),
            &format!("Reset task: {} after dependency restart", task.title),
        )?;
        reset_count += 1;
    }

    Ok(reset_count)
}
pub fn prepare_task_for_manual_restart(
    task: &mut CoordinationTask,
    now: DateTime<Utc>,
    new_generation: u32,
) -> Option<u32> {
    if !matches!(
        task.status,
        CoordinationTaskStatus::Failed | CoordinationTaskStatus::Canceled
    ) {
        return None;
    }

    let previous_generation = task.execution_generation;
    task.execution_generation = new_generation;
    task.dependency_generation = Some(new_generation);
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.artifact_summary = None;
    task.failure_kind = None;
    task.recovery_action = None;
    task.updated_at = now;
    if task.attempt_count >= task.max_attempts.max(1) {
        task.max_attempts = manual_restart_max_attempts(task);
    }
    Some(previous_generation)
}
const ABSOLUTE_MAX_ATTEMPTS: u32 = 10;
pub fn manual_restart_max_attempts(task: &CoordinationTask) -> u32 {
    task.attempt_count
        .saturating_add(2)
        .min(ABSOLUTE_MAX_ATTEMPTS)
}
pub fn format_attempt_suffix(task: &CoordinationTask) -> String {
    if task.max_attempts <= 1 {
        String::new()
    } else {
        format!(" (attempt {}/{})", task.attempt_count, task.max_attempts)
    }
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
    let mut lines = notes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with(&prefix))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !value.is_empty() {
        lines.push(format!("{key}={value}"));
    }
    lines.join("\n")
}
fn restart_target_dependency_ids(
    tasks: &[CoordinationTask],
    restart_target_ids: &BTreeSet<String>,
) -> Vec<String> {
    let mut dependency_ids = BTreeSet::new();
    for task in tasks {
        if !restart_target_ids.contains(&task.task_id) {
            continue;
        }
        for dependency in &task.dependencies {
            dependency_ids.insert(dependency.clone());
        }
    }
    dependency_ids.into_iter().collect()
}
fn post_failure_repair_prompt(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
    restart_target_ids: &BTreeSet<String>,
) -> String {
    let failures = tasks
        .iter()
        .filter(|task| restart_target_ids.contains(&task.task_id))
        .map(|task| {
            format!(
                "- {}: {}",
                task.title,
                task.artifact_summary
                    .as_deref()
                    .unwrap_or("failed without a recorded summary")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let task_doc_path = lightweight_current_task_doc_path(run);
    let build_and_test_hint = match lightweight_repo_configuration_for_run(run) {
        LightweightRepoConfiguration::Rust => {
            "Run `cargo build` and `cargo test` before reporting completion."
        }
        LightweightRepoConfiguration::Zig => {
            "Run `zig build` and `zig build test` before reporting completion."
        }
        LightweightRepoConfiguration::None => {
            "Run the repository's normal build and test commands before reporting completion."
        }
    };
    format!(
        "The previous implementation pass failed and needs a focused repository repair before the failed work retries.\n\nCurrent failed targets:\n{failures}\n\nRequirements:\n- Inspect the current repo state first, including any partial changes already left in the worktree.\n- Organize the repair into one coherent patch-level commit boundary for this task.\n- Fix the repo so the build and test path is healthy enough for the blocked implementation tasks to resume.\n- {build_and_test_hint}\n- Update code, config, scripts, or scaffolding only where needed to restore a working baseline.\n- Do not edit `{task_doc_path}`; review owns task-list progression and next-pass task-list creation.\n- Preserve repo-specific work unless changing it is necessary to clear the failure.\n\nReturn the normal write-task report format only."
    )
}
