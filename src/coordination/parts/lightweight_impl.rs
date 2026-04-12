use super::*;
pub const LIGHTWEIGHT_TASK_SPLITTER_NOTE_KEY: &str = "lightweight_task_splitter";
pub const LIGHTWEIGHT_WORKERS_NOTE_KEY: &str = "lightweight_worker_models";
pub const LIGHTWEIGHT_VALIDATION_NOTE_KEY: &str = "lightweight_validation_model";
pub const LIGHTWEIGHT_TASK_DOC_NOTE_KEY: &str = "lightweight_task_doc";
pub const LIGHTWEIGHT_TASK_DOC_CYCLE_NOTE_KEY: &str = "lightweight_task_doc_cycle";
pub const LIGHTWEIGHT_REPO_CONFIGURATION_NOTE_KEY: &str = "lightweight_repo_configuration";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LightweightRepoConfiguration {
    #[default]
    None,
    Rust,
    Zig,
}
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
pub fn lightweight_repo_configuration_for_run(
    run: &CoordinationRun,
) -> LightweightRepoConfiguration {
    lightweight_config_from_run(run).repo_configuration
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
