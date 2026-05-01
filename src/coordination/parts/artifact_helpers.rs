use super::*;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ArtifactContractMeta {
    pub payload_kind: TaskPayloadKind,
    pub contract_version: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct MaterializedArtifactFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, Default)]
pub struct WriteScopeSnapshot {
    pub file_hashes: BTreeMap<String, u64>,
}

pub fn artifact_contract_meta_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("contract.json")
}

pub fn persist_artifact_contract_meta(
    run_id: &str,
    task_id: &str,
    payload_kind: TaskPayloadKind,
    contract_version: u32,
) -> Result<()> {
    let meta = ArtifactContractMeta {
        payload_kind,
        contract_version,
    };
    fs::create_dir_all(artifact_dir(run_id, task_id))?;
    write_json_atomic(&artifact_contract_meta_path(run_id, task_id), &meta)
}

pub fn load_artifact_contract_meta(run_id: &str, task_id: &str) -> Option<ArtifactContractMeta> {
    let path = artifact_contract_meta_path(run_id, task_id);
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

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

pub fn workspace_relative_path(root: &Path, path: &str) -> Option<String> {
    let candidate = Path::new(path.trim());
    let relative = if candidate.is_absolute() {
        candidate.strip_prefix(root).ok()?.to_path_buf()
    } else {
        candidate.to_path_buf()
    };
    Some(relative.to_string_lossy().trim_matches('/').to_string())
}

pub fn scope_allows_relative_path(scope: &str, relative_path: &str) -> bool {
    let normalized_scope = normalize_scope(scope);
    if let Some(expanded) = expand_braces(&normalized_scope) {
        return expanded
            .iter()
            .any(|s| scope_allows_relative_path_inner(s, relative_path));
    }
    scope_allows_relative_path_inner(&normalized_scope, relative_path)
}

pub fn scope_allows_relative_path_inner(normalized_scope: &str, relative_path: &str) -> bool {
    let normalized_path = normalize_scope(relative_path);
    normalized_scope == "."
        || normalized_scope == "**"
        || normalized_scope == normalized_path
        || normalized_path.starts_with(&(normalized_scope.to_owned() + "/"))
        || wildcard_prefix(normalized_scope)
            .map(|prefix| normalized_path.starts_with(prefix.trim_matches('/')))
            .unwrap_or(false)
}

pub fn expand_braces(scope: &str) -> Option<Vec<String>> {
    let open = scope.find('{')?;
    let close = scope[open..].find('}').map(|i| i + open)?;
    let prefix = &scope[..open];
    let suffix = &scope[close + 1..];
    let alternatives = scope[open + 1..close].split(',');
    Some(
        alternatives
            .map(|alt| format!("{}{}{}", prefix, alt.trim(), suffix))
            .collect(),
    )
}

pub fn changed_file_exists(root: &Path, changed_file: &str) -> bool {
    let candidate = if Path::new(changed_file).is_absolute() {
        PathBuf::from(changed_file)
    } else {
        root.join(changed_file)
    };
    candidate.is_file()
}

pub fn snapshot_declared_write_scope(task: &CoordinationTask) -> WriteScopeSnapshot {
    WriteScopeSnapshot {
        file_hashes: collect_declared_write_scope_file_contents(task)
            .into_iter()
            .map(|(path, content)| (path, hash_materialized_content(&content)))
            .collect(),
    }
}

pub fn capture_materialized_files_from_snapshot(
    task: &CoordinationTask,
    snapshot: &WriteScopeSnapshot,
) -> Vec<MaterializedArtifactFile> {
    collect_declared_write_scope_file_contents(task)
        .into_iter()
        .filter_map(|(path, content)| {
            let hash = hash_materialized_content(&content);
            if snapshot.file_hashes.get(&path).copied() == Some(hash) {
                return None;
            }
            Some(MaterializedArtifactFile { path, content })
        })
        .collect()
}

pub fn capture_artifact_materialized_files(
    task: &CoordinationTask,
    artifact: &TaskArtifact,
) -> Vec<MaterializedArtifactFile> {
    if !task_mutates_workspace(task) {
        return Vec::new();
    }

    if artifact.report.changed_files.is_empty() {
        return collect_declared_write_scope_file_contents(task)
            .into_iter()
            .map(|(path, content)| MaterializedArtifactFile { path, content })
            .collect();
    }

    let materialization_root = task_runsc_materialization_root(task);
    artifact
        .report
        .changed_files
        .iter()
        .filter_map(|changed_file| {
            let candidate = if Path::new(changed_file).is_absolute() {
                PathBuf::from(changed_file)
            } else {
                materialization_root.join(changed_file)
            };
            let content = fs::read_to_string(&candidate).ok()?;
            Some(MaterializedArtifactFile {
                path: normalize_scope(changed_file),
                content,
            })
        })
        .collect()
}

pub fn collect_declared_write_scope_file_contents(
    task: &CoordinationTask,
) -> BTreeMap<String, String> {
    if !task_mutates_workspace(task) || task.write_scope.is_empty() {
        return BTreeMap::new();
    }

    let root = task_runsc_materialization_root(task);
    collect_write_scope_file_contents(&root, &task.write_scope)
}

pub fn collect_write_scope_file_contents(
    root: &Path,
    write_scope: &[String],
) -> BTreeMap<String, String> {
    if write_scope.is_empty() {
        return BTreeMap::new();
    }

    let mut files = BTreeMap::new();
    for scope in write_scope {
        collect_scope_file_contents(&root, scope, &mut files);
    }
    files
}

fn collect_scope_file_contents(root: &Path, scope: &str, files: &mut BTreeMap<String, String>) {
    if let Some(expanded) = expand_braces(scope) {
        for entry in expanded {
            collect_scope_file_contents(root, &entry, files);
        }
        return;
    }

    let normalized_scope = normalize_scope(scope);
    if normalized_scope.is_empty() {
        return;
    }

    if normalized_scope == "." || normalized_scope == "**" {
        collect_tree_file_contents(root, root, files);
        return;
    }

    if let Some(prefix) = wildcard_prefix(&normalized_scope) {
        let candidate = root.join(prefix.trim_matches('/'));
        collect_tree_file_contents(root, &candidate, files);
        return;
    }

    let candidate = root.join(&normalized_scope);
    if candidate.is_dir() {
        collect_tree_file_contents(root, &candidate, files);
    } else if candidate.is_file() {
        insert_materialized_file(root, &candidate, files);
    }
}

fn collect_tree_file_contents(root: &Path, candidate: &Path, files: &mut BTreeMap<String, String>) {
    let Ok(metadata) = fs::metadata(candidate) else {
        return;
    };
    if metadata.is_file() {
        insert_materialized_file(root, candidate, files);
        return;
    }
    if !metadata.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(candidate) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
                continue;
            }
            collect_tree_file_contents(root, &path, files);
        } else {
            insert_materialized_file(root, &path, files);
        }
    }
}

fn insert_materialized_file(root: &Path, candidate: &Path, files: &mut BTreeMap<String, String>) {
    let Ok(content) = fs::read_to_string(candidate) else {
        return;
    };
    let Ok(relative) = candidate.strip_prefix(root) else {
        return;
    };
    let path = normalize_scope(&relative.to_string_lossy());
    if path.is_empty() {
        return;
    }
    files.insert(path, content);
}

fn hash_materialized_content(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

pub fn persist_artifact_materialized_files(
    run_id: &str,
    task_id: &str,
    files: &[MaterializedArtifactFile],
) -> Result<()> {
    let path = artifact_materialized_files_path(run_id, task_id);
    if files.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    write_json_atomic(&path, &files.to_vec())
}

pub fn load_artifact_materialized_files(
    run_id: &str,
    task_id: &str,
) -> Vec<MaterializedArtifactFile> {
    let path = artifact_materialized_files_path(run_id, task_id);
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn changed_file_matches_output_path(changed_file: &str, output_path: &str) -> bool {
    let normalized_changed = normalize_scope(changed_file);
    let normalized_output = normalize_scope(output_path);
    if normalized_changed == normalized_output {
        return true;
    }

    prompt_track_identity(&normalized_changed)
        .zip(prompt_track_identity(&normalized_output))
        .is_some_and(|(left, right)| left == right)
}

fn prompt_track_identity(path: &str) -> Option<String> {
    let normalized = normalize_scope(path);
    let path = Path::new(&normalized);
    let parent = path.parent()?.to_string_lossy().replace('\\', "/");
    if !parent.ends_with("docs/autonomous/prompts") {
        return None;
    }

    let file_name = path.file_name()?.to_str()?;
    let mut parts = file_name.split('-');
    let prefix = parts.next()?;
    let track = parts.next()?;
    if prefix != "track" || track.is_empty() || !track.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(format!("{prefix}-{track}"))
}

pub fn make_task_session(task: &CoordinationTask) -> ChatSession {
    ChatSession {
        id: task.task_id.clone(),
        name: format!("auto-task-{}", task.title),
        system_prompt: None,
        json_summary: None,
        claude_session_id: None,
        minimax_session_id: None,
        qwen_session_id: None,
        kimi_session_id: None,
        codex_session_id: None,
        nvidia_session_id: None,
        gemini_session_id: None,
        openrouter_session_id: None,
        opencode_session_id: None,
        hermes_session_id: None,
        forge_session_id: None,
        claw_session_id: None,
        omniforge_session_id: None,
        omniforge_model_id: None,
        launched_from: Some(task.cwd.clone()),
        runsc_dirs: Vec::new(),
        runsc_image: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        messages: Vec::new(),
    }
}
