use super::*;
use fs2::FileExt;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

const WORKSPACE_ACTIVITY_SCAN_LIMIT: usize = 4_000;
const WORKSPACE_ACTIVITY_EVENT_PREVIEW: usize = 5;
const WORKSPACE_ACTIVITY_REPORT_INTERVAL_SECS: i64 = 5 * 60;

pub struct WorkspaceActivityObserver {
    workspace_root: PathBuf,
    known_files: BTreeMap<String, SystemTime>,
}

impl WorkspaceActivityObserver {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let known_files = scan_workspace_file_mtimes(&workspace_root).unwrap_or_default();
        Self {
            workspace_root,
            known_files,
        }
    }

    pub fn observe_and_maybe_emit_summary(
        &mut self,
        run_id: &str,
        task: &CoordinationTask,
        provider_name: &str,
    ) -> Result<()> {
        let current_files = scan_workspace_file_mtimes(&self.workspace_root)?;
        let changed = changed_workspace_paths(&self.known_files, &current_files);
        self.known_files = current_files;
        record_run_workspace_activity(run_id, task, provider_name, changed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RunWorkspaceActivityState {
    window_started_at: DateTime<Utc>,
    pending_paths: BTreeSet<String>,
}

fn workspace_activity_state_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("workspace-activity.json")
}

fn workspace_activity_lock_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("workspace-activity.lock")
}

fn record_run_workspace_activity(
    run_id: &str,
    task: &CoordinationTask,
    provider_name: &str,
    changed: Vec<String>,
) -> Result<()> {
    let now = Utc::now();
    let lock_path = workspace_activity_lock_path(run_id);
    let parent = lock_path
        .parent()
        .context("workspace activity lock missing parent")?;
    fs::create_dir_all(parent)?;
    let lock_file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("failed to open {}", lock_path.display()))?;
    lock_file
        .lock_exclusive()
        .with_context(|| format!("failed to lock {}", lock_path.display()))?;

    let state_path = workspace_activity_state_path(run_id);
    let mut state =
        load_workspace_activity_state(&state_path).unwrap_or(RunWorkspaceActivityState {
            window_started_at: now,
            pending_paths: BTreeSet::new(),
        });
    let summary = update_workspace_activity_state(&mut state, now, changed);
    write_json_atomic(&state_path, &state)?;
    lock_file
        .unlock()
        .with_context(|| format!("failed to unlock {}", lock_path.display()))?;

    if let Some(message) = summary {
        append_event_with_provider(
            run_id,
            "run_workspace_activity_summary",
            None,
            Some(provider_name),
            task.task_category,
            None,
            &message,
        )?;
    }

    Ok(())
}

fn load_workspace_activity_state(path: &Path) -> Option<RunWorkspaceActivityState> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn update_workspace_activity_state(
    state: &mut RunWorkspaceActivityState,
    now: DateTime<Utc>,
    changed: Vec<String>,
) -> Option<String> {
    state.pending_paths.extend(changed);
    if now
        .signed_duration_since(state.window_started_at)
        .num_seconds()
        < WORKSPACE_ACTIVITY_REPORT_INTERVAL_SECS
    {
        return None;
    }

    let summary = summarize_workspace_activity_paths(&state.pending_paths);
    state.window_started_at = now;
    state.pending_paths.clear();
    Some(summary)
}

fn summarize_workspace_activity_paths(paths: &BTreeSet<String>) -> String {
    if paths.is_empty() {
        return "No non-ignored workspace file changes observed in the last 5 minutes.".to_string();
    }

    let preview = paths
        .iter()
        .take(WORKSPACE_ACTIVITY_EVENT_PREVIEW)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let extra_count = paths.len().saturating_sub(WORKSPACE_ACTIVITY_EVENT_PREVIEW);
    let suffix = if extra_count == 0 {
        String::new()
    } else {
        format!(" (+{} more)", extra_count)
    };
    format!(
        "Workspace file changes observed in the last 5 minutes: {}{}.",
        preview, suffix
    )
}

fn changed_workspace_paths(
    previous: &BTreeMap<String, SystemTime>,
    current: &BTreeMap<String, SystemTime>,
) -> Vec<String> {
    current
        .iter()
        .filter_map(|(path, modified_at)| match previous.get(path) {
            Some(previous_modified_at) if previous_modified_at >= modified_at => None,
            _ => Some(path.clone()),
        })
        .collect()
}

fn scan_workspace_file_mtimes(root: &Path) -> Result<BTreeMap<String, SystemTime>> {
    if !root.is_dir() {
        return Ok(BTreeMap::new());
    }

    let mut candidates = Vec::new();
    collect_workspace_file_candidates(root, root, &mut candidates, WORKSPACE_ACTIVITY_SCAN_LIMIT)?;
    let visible_candidates = filter_gitignored_paths(root, candidates)?;
    let mut mtimes = BTreeMap::new();
    for relative in visible_candidates {
        let path = root.join(&relative);
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        let Ok(modified_at) = metadata.modified() else {
            continue;
        };
        mtimes.insert(relative, modified_at);
    }
    Ok(mtimes)
}

fn collect_workspace_file_candidates(
    root: &Path,
    current: &Path,
    files: &mut Vec<String>,
    limit: usize,
) -> Result<()> {
    if files.len() >= limit {
        return Ok(());
    }

    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        if files.len() >= limit {
            break;
        }
        let path = entry.path();
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if should_skip_workspace_activity_dir(relative) {
                continue;
            }
            collect_workspace_file_candidates(root, &path, files, limit)?;
        } else if file_type.is_file() {
            files.push(relative.to_string_lossy().into_owned());
        }
    }

    Ok(())
}

fn filter_gitignored_paths(root: &Path, files: Vec<String>) -> Result<Vec<String>> {
    if files.is_empty() {
        return Ok(files);
    }

    let mut child = match Command::new("git")
        .args(["check-ignore", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return Ok(files),
    };

    {
        let Some(mut stdin) = child.stdin.take() else {
            return Ok(files);
        };
        for file in &files {
            writeln!(stdin, "{file}")?;
        }
    }

    let output = child.wait_with_output()?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Ok(files);
    }

    let ignored_output = String::from_utf8_lossy(&output.stdout);
    let ignored = ignored_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<BTreeSet<_>>();
    if ignored.is_empty() {
        return Ok(files);
    }

    Ok(files
        .into_iter()
        .filter(|file| !ignored.contains(file.as_str()))
        .collect())
}

fn should_skip_workspace_activity_dir(relative: &Path) -> bool {
    matches!(
        relative.file_name().and_then(|name| name.to_str()),
        Some(
            ".git"
                | "target"
                | ".zig-cache"
                | "zig-out"
                | "node_modules"
                | "dist"
                | ".claude-desktop"
                | ".rag_model_cache"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    fn init_git_repo(path: &Path) {
        let status = Command::new("git")
            .args(["init"])
            .current_dir(path)
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn observer_reports_only_new_non_ignored_changes() {
        let workspace = tempfile::tempdir().unwrap();
        init_git_repo(workspace.path());
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::create_dir_all(workspace.path().join("target/debug")).unwrap();
        fs::create_dir_all(workspace.path().join(".zig-cache/h")).unwrap();
        fs::create_dir_all(workspace.path().join("ignored-dir")).unwrap();
        fs::write(
            workspace.path().join(".gitignore"),
            "ignored.txt\nignored-dir/\n",
        )
        .unwrap();
        fs::write(workspace.path().join("src/main.zig"), "const x = 1;\n").unwrap();
        fs::write(workspace.path().join("ignored.txt"), "old\n").unwrap();
        fs::write(workspace.path().join("target/debug/build.log"), "old\n").unwrap();
        fs::write(workspace.path().join(".zig-cache/h/cache.txt"), "old\n").unwrap();

        let mut observer = WorkspaceActivityObserver::new(workspace.path());
        thread::sleep(Duration::from_millis(1100));

        fs::write(workspace.path().join("src/main.zig"), "const x = 2;\n").unwrap();
        fs::write(workspace.path().join("ignored.txt"), "new\n").unwrap();
        fs::write(workspace.path().join("ignored-dir/hidden.txt"), "new\n").unwrap();
        fs::write(workspace.path().join("target/debug/build.log"), "new\n").unwrap();
        fs::write(workspace.path().join(".zig-cache/h/cache.txt"), "new\n").unwrap();

        let changed = changed_workspace_paths(
            &observer.known_files,
            &scan_workspace_file_mtimes(workspace.path()).unwrap(),
        );
        assert_eq!(changed, vec!["src/main.zig".to_string()]);

        observer.known_files = scan_workspace_file_mtimes(workspace.path()).unwrap();
        let changed_again = changed_workspace_paths(
            &observer.known_files,
            &scan_workspace_file_mtimes(workspace.path()).unwrap(),
        );
        assert!(changed_again.is_empty());
    }

    #[test]
    fn observer_detects_new_files() {
        let workspace = tempfile::tempdir().unwrap();
        init_git_repo(workspace.path());
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(workspace.path().join("src/main.zig"), "const x = 1;\n").unwrap();

        let observer = WorkspaceActivityObserver::new(workspace.path());
        thread::sleep(Duration::from_millis(1100));

        fs::write(workspace.path().join("src/new_file.zig"), "const y = 2;\n").unwrap();

        let changed = changed_workspace_paths(
            &observer.known_files,
            &scan_workspace_file_mtimes(workspace.path()).unwrap(),
        );
        assert_eq!(changed, vec!["src/new_file.zig".to_string()]);
    }

    #[test]
    fn workspace_activity_summary_reports_none_when_window_is_idle() {
        let start = Utc::now();
        let mut state = RunWorkspaceActivityState {
            window_started_at: start,
            pending_paths: BTreeSet::new(),
        };

        let summary = update_workspace_activity_state(
            &mut state,
            start + chrono::Duration::seconds(WORKSPACE_ACTIVITY_REPORT_INTERVAL_SECS),
            Vec::new(),
        );

        assert_eq!(
            summary.as_deref(),
            Some("No non-ignored workspace file changes observed in the last 5 minutes.")
        );
        assert!(state.pending_paths.is_empty());
    }

    #[test]
    fn workspace_activity_summary_aggregates_changed_paths() {
        let start = Utc::now();
        let mut state = RunWorkspaceActivityState {
            window_started_at: start,
            pending_paths: BTreeSet::new(),
        };

        assert!(update_workspace_activity_state(
            &mut state,
            start + chrono::Duration::seconds(60),
            vec!["src/main.zig".to_string(), "build.zig".to_string()],
        )
        .is_none());

        let summary = update_workspace_activity_state(
            &mut state,
            start + chrono::Duration::seconds(WORKSPACE_ACTIVITY_REPORT_INTERVAL_SECS),
            vec!["src/cli/main.zig".to_string()],
        );

        assert_eq!(
            summary.as_deref(),
            Some(
                "Workspace file changes observed in the last 5 minutes: build.zig, src/cli/main.zig, src/main.zig."
            )
        );
        assert!(state.pending_paths.is_empty());
    }
}
