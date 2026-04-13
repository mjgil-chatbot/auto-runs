use super::*;
pub const LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY: &str = "lightweight_task_splitter";
pub const LIGHTWEIGHT_WORKERS_NOTE_KEY: &str = "lightweight_worker_models";
pub const LIGHTWEIGHT_VALIDATION_NOTE_KEY: &str = "lightweight_validation_model";
pub const LIGHTWEIGHT_TASK_DOC_NOTE_KEY: &str = "lightweight_task_doc";
pub const LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY: &str = "lightweight_task_doc_cycle";
pub const LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY: &str = "lightweight_repo_configuration";
const LIGHTWEIGHT_PUBLISH_RECOVERY_MARKER: &str = "Git publish recovery context:";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightweightRepoConfiguration {
    #[default]
    None,
    Rust,
    Zig,
}
impl LightweightRepoConfiguration {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::None => "N/A",
            Self::Rust => "Rust",
            Self::Zig => "Zig",
        }
    }}
impl LightweightRepoConfiguration {
    fn note_value(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Rust => "rust",
            Self::Zig => "zig",
        }
    }}
impl LightweightRepoConfiguration {
    fn from_note_value(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "n/a" | "na" | "none" => Some(Self::None),
            "rust" => Some(Self::Rust),
            "zig" => Some(Self::Zig),
            _ => None,
        }
    }}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LightweightImplementationConfig {
    pub task_splitter_provider: String,
    pub validation_provider: String,
    pub worker_providers: Vec<String>,
    pub task_doc_path: String,
    pub task_doc_cycle: u32,
    pub repo_configuration: LightweightRepoConfiguration,
}
pub struct LightweightWorkflowBehavior<'a> {
    run: &'a CoordinationRun,
}
impl<'a> LightweightWorkflowBehavior<'a> {
    pub fn for_run(run: &'a CoordinationRun) -> Option<Self> {
        uses_lightweight_runtime_mode(&run.workflow_mode).then_some(Self { run })
    }}
impl<'a> LightweightWorkflowBehavior<'a> {
    pub fn claim_provider(
        &self,
        task: &CoordinationTask,
        default_provider: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<String> {
        let provider = task
            .assigned_provider
            .clone()
            .unwrap_or_else(|| default_provider.to_string());
        is_provider_on_cooldown(self.run, &provider, now)
            .is_none()
            .then_some(provider)
    }}
impl<'a> LightweightWorkflowBehavior<'a> {
    pub fn skips_artifact_acceptance(&self) -> bool {
        true
    }}
impl<'a> LightweightWorkflowBehavior<'a> {
    pub fn tracks_changed_files(&self) -> bool {
        false
    }}
impl<'a> LightweightWorkflowBehavior<'a> {
    pub fn persists_materialized_files(&self) -> bool {
        false
    }}
pub fn lightweight_config_from_run(run: &CoordinationRun) -> LightweightImplementationConfig {
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        return LightweightImplementationConfig {
            task_splitter_provider: run.provider.clone(),
            validation_provider: run.provider.clone(),
            worker_providers: vec![run.provider.clone()],
            task_doc_path: lightweight_default_task_doc_path(&run.design_doc_path),
            task_doc_cycle: 0,
            repo_configuration: LightweightRepoConfiguration::None,
        };
    }
    let task_splitter_provider = note_setting_value(&run.notes, LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY)
        .and_then(|value| normalize_provider_name(Some(value)))
        .unwrap_or_else(|| "codex".to_string());
    let worker_providers = parse_lightweight_worker_providers(note_setting_value(
        &run.notes,
        LIGHTWEIGHT_WORKERS_NOTE_KEY,
    ))
    .unwrap_or_else(|| vec![task_splitter_provider.clone()]);
    let validation_provider = note_setting_value(&run.notes, LIGHTWEIGHT_VALIDATION_NOTE_KEY)
        .and_then(|value| normalize_provider_name(Some(value)))
        .unwrap_or_else(|| task_splitter_provider.clone());
    let task_doc_path = note_setting_value(&run.notes, LIGHTWEIGHT_TASK_DOC_NOTE_KEY)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| lightweight_default_task_doc_path(&run.design_doc_path));
    let task_doc_path = normalize_workspace_local_path(&task_doc_path, &run.workspace_path);
    let task_doc_cycle = note_setting_value(&run.notes, LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY)
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    let repo_configuration =
        note_setting_value(&run.notes, LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY)
            .and_then(LightweightRepoConfiguration::from_note_value)
            .unwrap_or_default();

    LightweightImplementationConfig {
        task_splitter_provider,
        validation_provider,
        worker_providers,
        task_doc_path,
        task_doc_cycle,
        repo_configuration,
    }
}
pub fn normalize_workspace_local_path(path: &str, workspace_path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if raw_scope_is_absolute(trimmed) {
        let workspace = Path::new(workspace_path.trim());
        if workspace.is_absolute() {
            if let Ok(relative) = Path::new(trimmed).strip_prefix(workspace) {
                return normalize_scope(&relative.to_string_lossy().replace('\\', "/"));
            }
        }
        return trimmed.to_string();
    }

    normalize_scope(trimmed)
}
pub fn lightweight_default_task_doc_path(design_doc_path: &str) -> String {
    let design_path = Path::new(design_doc_path);
    let parent = design_path.parent();
    let stem = design_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("lightweight-implementation-doc");
    let task_file_name = format!("{stem}-tasks.md");
    match parent {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(task_file_name).to_string_lossy().into_owned()
        }
        _ => task_file_name,
    }
}
pub fn lightweight_notes_with_settings(
    notes: &str,
    task_doc_path: &str,
    task_splitter_provider: &str,
    worker_providers: &[String],
) -> String {
    let mut updated = notes.to_string();
    updated = notes_with_setting(
        &updated,
        LIGHTWEIGHT_TASK_DOC_NOTE_KEY,
        task_doc_path.trim(),
    );
    updated = notes_with_setting(
        &updated,
        LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY,
        task_splitter_provider.trim(),
    );
    updated = notes_with_setting(
        &updated,
        LIGHTWEIGHT_WORKERS_NOTE_KEY,
        &worker_providers.join(","),
    );
    notes_with_setting(&updated, LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY, "0")
}
pub fn lightweight_notes_with_repo_configuration(
    notes: &str,
    repo_configuration: LightweightRepoConfiguration,
) -> String {
    notes_with_setting(
        notes,
        LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY,
        repo_configuration.note_value(),
    )
}
pub fn lightweight_notes_with_validation_provider(
    notes: &str,
    validation_provider: &str,
) -> String {
    notes_with_setting(
        notes,
        LIGHTWEIGHT_VALIDATION_NOTE_KEY,
        validation_provider.trim(),
    )
}
pub fn lightweight_publish_failure_should_respawn(
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> bool {
    run.workflow_mode == AutonomousWorkflowMode::LightweightImplementation
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
        && task.attempt_count < task.max_attempts.max(1)
}
pub fn lightweight_publish_failure_prompt(prompt: &str, failure_message: &str) -> String {
    let base = prompt
        .split_once(LIGHTWEIGHT_PUBLISH_RECOVERY_MARKER)
        .map(|(prefix, _)| prefix.trim_end())
        .unwrap_or_else(|| prompt.trim_end());
    format!(
        "{base}\n\n{marker}\n- The previous worker turn already wrote repo changes locally, but the runtime's automatic git commit or push failed.\n- Start from the current worktree state, inspect the git failure below, and make only the smallest additional changes needed so commit and push succeed.\n- Preserve existing staged or unstaged changes unless changing them is required to clear the publish blocker.\n- Rerun the relevant local checks before reporting completion.\n- If the intended local commit already exists, resolve the remaining push blocker without unnecessary code churn.\nLatest automatic git publish failure:\n{failure_message}",
        marker = LIGHTWEIGHT_PUBLISH_RECOVERY_MARKER,
    )
}
pub fn lightweight_task_has_publish_recovery(task: &CoordinationTask) -> bool {
    task.prompt.contains(LIGHTWEIGHT_PUBLISH_RECOVERY_MARKER)
}
pub fn lightweight_generated_task_provider(
    run: &CoordinationRun,
    provider: Option<&str>,
    index: usize,
) -> Option<String> {
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        return Some(run.provider.clone());
    }
    if let Some(provider) = normalize_provider_name(provider) {
        return Some(provider);
    }

    let providers = lightweight_config_from_run(run).worker_providers;
    providers.get(index % providers.len()).cloned()
}
pub fn lightweight_task_splitter_provider(run: &CoordinationRun) -> String {
    lightweight_config_from_run(run).task_splitter_provider
}
pub fn lightweight_current_task_doc_path(run: &CoordinationRun) -> String {
    lightweight_config_from_run(run).task_doc_path
}
pub fn lightweight_next_task_doc_path_for_run(run: &CoordinationRun) -> String {
    let config = lightweight_config_from_run(run);
    let current_cycle =
        lightweight_task_doc_cycle_for_path(&config.task_doc_path).unwrap_or(config.task_doc_cycle);
    lightweight_next_task_doc_path(&config.task_doc_path, current_cycle + 1)
}
pub fn lightweight_status_doc_path_for_run(run: &CoordinationRun) -> String {
    lightweight_status_doc_path(&lightweight_current_task_doc_path(run))
}
fn lightweight_status_doc_path(task_doc_path: &str) -> String {
    let path = Path::new(task_doc_path);
    let parent = path.parent();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("lightweight-tasks");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("md");
    let file_name = format!("{stem}-status.{extension}");
    match parent {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(file_name).to_string_lossy().into_owned()
        }
        _ => file_name,
    }
}
fn parse_markdown_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let title = trimmed[level..].trim();
    (!title.is_empty()).then_some((level, title))
}
fn parse_markdown_task_item(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("- [ ] ")
        .or_else(|| trimmed.strip_prefix("* [ ] "))
        .or_else(|| trimmed.strip_prefix("+ [ ] "))
        .map(|title| (false, title))
        .or_else(|| trimmed.strip_prefix("- [x] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("* [x] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("+ [x] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("- [X] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("* [X] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("+ [X] ").map(|title| (true, title)))
        .or_else(|| trimmed.strip_prefix("- ").map(|title| (false, title)))
        .or_else(|| trimmed.strip_prefix("* ").map(|title| (false, title)))
        .or_else(|| trimmed.strip_prefix("+ ").map(|title| (false, title)))
        .or_else(|| parse_numbered_markdown_item(trimmed).map(|title| (false, title)))
}
fn parse_numbered_markdown_item(line: &str) -> Option<&str> {
    let digits = line.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    line[digits..]
        .strip_prefix(". ")
        .or_else(|| line[digits..].strip_prefix(") "))
}
fn lightweight_source_task_base_id(section_path: &[String], title: &str) -> String {
    let mut parts = section_path
        .iter()
        .map(|segment| lightweight_source_task_slug(segment))
        .collect::<Vec<_>>();
    parts.push(lightweight_source_task_slug(title));
    format!("source-task-{}", parts.join("--"))
}
fn lightweight_source_task_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in value.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}
fn lightweight_task_status_label(status: CoordinationTaskStatus) -> &'static str {
    match status {
        CoordinationTaskStatus::Queued => "queued",
        CoordinationTaskStatus::Claiming => "claiming",
        CoordinationTaskStatus::Running => "running",
        CoordinationTaskStatus::ArtifactReady => "artifact_ready",
        CoordinationTaskStatus::Blocked => "blocked",
        CoordinationTaskStatus::Completed => "completed",
        CoordinationTaskStatus::Failed => "failed",
        CoordinationTaskStatus::Canceled => "canceled",
    }
}
fn lightweight_task_kind_label(kind: CoordinationTaskKind) -> &'static str {
    generated_task_label(&kind)
}
fn lightweight_recovery_action_label(action: TaskRecoveryAction) -> &'static str {
    match action {
        TaskRecoveryAction::None => "none",
        TaskRecoveryAction::RetryTask => "retry_task",
        TaskRecoveryAction::RetryPublish => "retry_publish",
        TaskRecoveryAction::ReconcileState => "reconcile_state",
        TaskRecoveryAction::ManualIntervention => "manual_intervention",
    }
}
pub fn lightweight_repo_configuration_for_run(
    run: &CoordinationRun,
) -> LightweightRepoConfiguration {
    lightweight_config_from_run(run).repo_configuration
}
pub fn lightweight_review_write_scope(run: &CoordinationRun) -> Vec<String> {
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        return Vec::new();
    }
    if !run.allow_write_tasks {
        return Vec::new();
    }
    vec![
        run.design_doc_path.clone(),
        lightweight_current_task_doc_path(run),
        lightweight_next_task_doc_path_for_run(run),
    ]
}
pub fn lightweight_review_task_is_superseded(
    tasks: &[CoordinationTask],
    task: &CoordinationTask,
) -> bool {
    task.kind == CoordinationTaskKind::ReviewArtifacts
        && tasks.iter().any(|candidate| {
            candidate.kind == CoordinationTaskKind::ReviewArtifacts
                && candidate.parent_task_id.as_deref() == Some(task.task_id.as_str())
        })
}
pub fn lightweight_review_has_actionable_repo_local_work(artifact: &TaskArtifact) -> bool {
    artifact
        .report
        .fix_tasks
        .iter()
        .any(|spec| match parse_generated_kind(&spec.kind) {
            Ok(CoordinationTaskKind::GeneratedWriteTask) => true,
            Ok(_) => false,
            Err(_) => true,
        })
}
fn lightweight_blocker_text_signature(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn lightweight_task_omits_changed_files(
    run: &CoordinationRun,
    _task: &CoordinationTask,
) -> bool {
    LightweightWorkflowBehavior::for_run(run)
        .map(|behavior| !behavior.tracks_changed_files())
        .unwrap_or(false)
}
pub fn lightweight_generated_write_claims_ignore_scope_conflicts(
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> bool {
    uses_lightweight_runtime_mode(&run.workflow_mode)
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
}
pub fn lightweight_write_scope_conflict_is_coordination_only(
    run: &CoordinationRun,
    candidate_task: &CoordinationTask,
    candidate_scopes: &[String],
    other_task: &CoordinationTask,
) -> bool {
    if !uses_lightweight_runtime_mode(&run.workflow_mode)
        || candidate_task.kind != CoordinationTaskKind::GeneratedWriteTask
        || other_task.kind != CoordinationTaskKind::GeneratedWriteTask
    {
        return false;
    }

    let task_doc_path = normalize_scope(&lightweight_current_task_doc_path(run));
    let candidate_touches_task_doc = candidate_scopes
        .iter()
        .any(|scope| scope_allows_relative_path(scope, &task_doc_path));
    let other_touches_task_doc = other_task
        .write_scope
        .iter()
        .any(|scope| scope_allows_relative_path(scope, &task_doc_path));
    if !(candidate_touches_task_doc && other_touches_task_doc) {
        return false;
    }

    let filtered_candidate = candidate_scopes
        .iter()
        .filter(|scope| normalize_scope(scope) != task_doc_path)
        .cloned()
        .collect::<Vec<_>>();
    let filtered_other = other_task
        .write_scope
        .iter()
        .filter(|scope| normalize_scope(scope) != task_doc_path)
        .cloned()
        .collect::<Vec<_>>();
    !scopes_conflict(&filtered_candidate, &filtered_other)
}
pub fn lightweight_validate_config(
    task_doc_path: &str,
    task_splitter_provider: &str,
    validation_provider: &str,
    worker_providers: &[String],
) -> Result<()> {
    if task_doc_path.trim().is_empty() {
        anyhow::bail!("lightweight_task_doc is required for lightweight implementation runs");
    }

    if normalize_provider_name(Some(task_splitter_provider)).is_none() {
        anyhow::bail!(
            "unknown lightweight task splitter model '{}'",
            task_splitter_provider
        );
    }
    if normalize_provider_name(Some(validation_provider)).is_none() {
        anyhow::bail!(
            "unknown lightweight validation model '{}'",
            validation_provider
        );
    }
    if worker_providers.is_empty() {
        anyhow::bail!("at least one lightweight worker model is required");
    }
    for provider in worker_providers {
        if normalize_provider_name(Some(provider)).is_none() {
            anyhow::bail!("unknown lightweight worker model '{}'", provider);
        }
    }
    Ok(())
}
pub fn uses_lightweight_runtime_mode(mode: &AutonomousWorkflowMode) -> bool {
    matches!(
        mode,
        AutonomousWorkflowMode::LightweightImplementation | AutonomousWorkflowMode::GithubBranchFix
    )
}
pub fn lightweight_runtime_note_setting_value<'a>(notes: &'a str, key: &str) -> Option<&'a str> {
    note_setting_value(notes, key)
}
pub fn lightweight_runtime_notes_with_setting(notes: &str, key: &str, value: &str) -> String {
    notes_with_setting(notes, key, value)
}
pub fn validate_lightweight_notes(notes: &str, design_doc_path: &str) -> Result<()> {
    let task_doc_path = note_setting_value(notes, LIGHTWEIGHT_TASK_DOC_NOTE_KEY)
        .map(str::to_string)
        .unwrap_or_else(|| lightweight_default_task_doc_path(design_doc_path));
    let task_splitter_provider =
        note_setting_value(notes, LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY).unwrap_or("codex");
    let worker_providers =
        parse_lightweight_worker_providers(note_setting_value(notes, LIGHTWEIGHT_WORKERS_NOTE_KEY))
            .unwrap_or_else(|| vec![task_splitter_provider.to_string()]);
    let validation_provider = note_setting_value(notes, LIGHTWEIGHT_VALIDATION_NOTE_KEY)
        .unwrap_or(task_splitter_provider);
    if let Some(value) = note_setting_value(notes, LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY) {
        if LightweightRepoConfiguration::from_note_value(value).is_none() {
            anyhow::bail!("unknown lightweight repo configuration '{}'", value);
        }
    }

    lightweight_validate_config(
        &task_doc_path,
        task_splitter_provider,
        validation_provider,
        &worker_providers,
    )
}
pub fn parse_lightweight_worker_providers(value: Option<&str>) -> Option<Vec<String>> {
    let parsed = value
        .unwrap_or_default()
        .split([',', '\n'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| normalize_provider_name(Some(entry)))
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}
pub fn lightweight_prompt_prefix(task_doc_path: &str) -> String {
    format!(
        "The active task list document path for this run is `{}`. Generated implementation workers must treat that document as read-only. Do not edit, rewrite, reorder, delete, summarize, or regenerate it from generated worker tasks. Review owns task-list progression and next-pass task-list updates.",
        task_doc_path
    )
}
pub fn lightweight_generated_task_prompt(
    run: &CoordinationRun,
    _kind: &CoordinationTaskKind,
    prompt: &str,
) -> String {
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        return github_branch_fix_generated_task_prompt(run, prompt);
    }
    format!(
        "{}\n\n{}",
        lightweight_prompt_prefix(&lightweight_current_task_doc_path(run)),
        prompt
    )
}
pub fn lightweight_review_prompt(run: &CoordinationRun) -> String {
    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        return github_branch_fix_review_prompt(run);
    }
    let current_task_doc_path = lightweight_current_task_doc_path(run);
    let next_task_doc_path = lightweight_next_task_doc_path_for_run(run);
    format!(
        "{}\n\nValidate the completed implementation against the design doc `{}` and the active task list `{}`.\n\nTreat the active task list as read-only evidence for the current pass.\n\nIf more work remains after a full reassessment of the repo state:\n- update the canonical design doc `{}` if the reassessment changes the plan or architecture materially\n- create the next task list document at `{}`\n- explain the remaining gaps\n- emit smaller follow-up fix tasks so implementation workers can continue from that new task list\n\nIf the implementation is complete:\n- do not create a new task list document\n- emit no fix tasks\n- mark the work as complete based on the current task list and workspace state.",
        lightweight_prompt_prefix(&current_task_doc_path),
        run.design_doc_path,
        current_task_doc_path,
        run.design_doc_path,
        next_task_doc_path
    )
}
fn lightweight_next_task_doc_path(current_path: &str, next_cycle: u32) -> String {
    let path = Path::new(current_path);
    let parent = path.parent();
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("lightweight-tasks");
    let stem = stem
        .rsplit_once("-pass-")
        .filter(|(_, suffix)| suffix.chars().all(|ch| ch.is_ascii_digit()))
        .map(|(prefix, _)| prefix)
        .unwrap_or(stem);
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("md");
    let file_name = format!("{stem}-pass-{next_cycle}.{extension}");
    match parent {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(file_name).to_string_lossy().into_owned()
        }
        _ => file_name,
    }
}
fn lightweight_task_doc_cycle_for_path(task_doc_path: &str) -> Option<u32> {
    let stem = Path::new(task_doc_path)
        .file_stem()
        .and_then(|value| value.to_str())?;
    let (_, suffix) = stem.rsplit_once("-pass-")?;
    suffix.parse::<u32>().ok()
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
