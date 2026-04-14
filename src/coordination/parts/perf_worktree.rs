use super::*;

const PERF_WORKTREE_ROOT_NOTE_KEY: &str = "perf_worktree_root";
const PERF_WORKTREE_PATH_NOTE_KEY: &str = "perf_worktree_path";

pub fn perf_worktree_root_for_run(run: &CoordinationRun) -> Option<String> {
    note_setting_value(&run.notes, PERF_WORKTREE_ROOT_NOTE_KEY).map(str::to_string)
}

pub fn perf_workspace_path_for_run(run: &CoordinationRun) -> String {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return run.workspace_path.clone();
    }

    note_setting_value(&run.notes, PERF_WORKTREE_PATH_NOTE_KEY)
        .map(str::to_string)
        .unwrap_or_else(|| run.workspace_path.clone())
}

pub fn prepare_perf_workspace(run: &mut CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(());
    }

    if perf_worktree_root_for_run(run).is_some()
        && note_setting_value(&run.notes, PERF_WORKTREE_PATH_NOTE_KEY).is_some()
    {
        if perf_workspace_is_valid(run) {
            return Ok(());
        }
        clear_stale_perf_workspace(run)?;
    }

    let workspace = PathBuf::from(run.workspace_path.trim());
    let canonical_workspace = workspace.canonicalize().ok().unwrap_or(workspace);
    let repo_root = git_repo_root_for_workspace(&canonical_workspace)?;
    let relative_workspace = canonical_workspace
        .strip_prefix(&repo_root)
        .unwrap_or(Path::new(""));
    let worktree_root = coordination_root_dir()
        .join("perf-worktrees")
        .join(&run.run_id);
    let parent = worktree_root
        .parent()
        .context("perf worktree path missing parent directory")?;
    fs::create_dir_all(parent)?;

    let worktree_root_text = worktree_root.to_string_lossy().into_owned();
    run_git_command(
        &repo_root,
        &["worktree", "add", "--detach", &worktree_root_text, "HEAD"],
    )
    .with_context(|| {
        format!(
            "failed to create disposable perf worktree for {}",
            repo_root.display()
        )
    })?;

    let effective_workspace = if relative_workspace.as_os_str().is_empty() {
        worktree_root.clone()
    } else {
        worktree_root.join(relative_workspace)
    };
    rewrite_local_dependency_paths_in_worktree(&repo_root, &worktree_root)?;
    let effective_workspace_text = effective_workspace.to_string_lossy().into_owned();

    run.notes = notes_with_setting(&run.notes, PERF_WORKTREE_ROOT_NOTE_KEY, &worktree_root_text);
    run.notes = notes_with_setting(
        &run.notes,
        PERF_WORKTREE_PATH_NOTE_KEY,
        &effective_workspace_text,
    );
    Ok(())
}

pub fn ensure_perf_workspace_ready(run: &mut CoordinationRun) -> Result<bool> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(false);
    }

    if perf_workspace_is_valid(run) {
        return Ok(false);
    }

    clear_stale_perf_workspace(run)?;
    prepare_perf_workspace(run)?;
    Ok(true)
}

pub fn cleanup_perf_workspace(run: &CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(());
    }

    let Some(worktree_root) = perf_worktree_root_for_run(run) else {
        return Ok(());
    };
    let worktree_root_path = PathBuf::from(&worktree_root);
    if !worktree_root_path.exists() {
        return Ok(());
    }

    let workspace = PathBuf::from(run.workspace_path.trim());
    let canonical_workspace = workspace.canonicalize().ok().unwrap_or(workspace);
    let repo_root = git_repo_root_for_workspace(&canonical_workspace)?;
    let remove_result = run_git_command(
        &repo_root,
        &["worktree", "remove", "--force", &worktree_root],
    );
    if worktree_root_path.exists() {
        fs::remove_dir_all(&worktree_root_path).with_context(|| {
            format!(
                "failed to remove disposable perf worktree {}",
                worktree_root_path.display()
            )
        })?;
    }
    remove_result.map(|_| ())
}

fn perf_workspace_is_valid(run: &CoordinationRun) -> bool {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return true;
    }

    let Some(worktree_root) = perf_worktree_root_for_run(run) else {
        return false;
    };
    let Some(workspace_path) = note_setting_value(&run.notes, PERF_WORKTREE_PATH_NOTE_KEY) else {
        return false;
    };
    let worktree_root_path = Path::new(&worktree_root);
    let workspace_path = Path::new(workspace_path);
    worktree_root_path.is_dir() && workspace_path.is_dir() && path_is_git_workspace(workspace_path)
}

fn clear_stale_perf_workspace(run: &mut CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(());
    }

    let workspace = PathBuf::from(run.workspace_path.trim());
    let canonical_workspace = workspace.canonicalize().ok().unwrap_or(workspace);
    let repo_root = git_repo_root_for_workspace(&canonical_workspace)?;

    if let Some(worktree_root) = perf_worktree_root_for_run(run) {
        let worktree_root_path = PathBuf::from(&worktree_root);
        let _ = run_git_command(
            &repo_root,
            &["worktree", "remove", "--force", &worktree_root],
        );
        let _ = run_git_command(&repo_root, &["worktree", "prune", "--expire", "now"]);
        if worktree_root_path.exists() {
            let _ = fs::remove_dir_all(&worktree_root_path);
        }
    }

    run.notes = notes_with_setting(&run.notes, PERF_WORKTREE_ROOT_NOTE_KEY, "");
    run.notes = notes_with_setting(&run.notes, PERF_WORKTREE_PATH_NOTE_KEY, "");
    Ok(())
}

pub fn validate_perf_workspace_input(input: &UpsertRunInput) -> Result<()> {
    if input.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return Ok(());
    }

    let workspace = PathBuf::from(input.workspace_path.trim());
    let canonical_workspace = workspace.canonicalize().ok().unwrap_or(workspace);
    let _ = git_repo_root_for_workspace(&canonical_workspace).with_context(|| {
        format!(
            "PerfHillClimber workspace_path {} must be inside a git repository",
            canonical_workspace.display()
        )
    })?;
    Ok(())
}

fn git_repo_root_for_workspace(workspace: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .with_context(|| format!("failed to run git in {}", workspace.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git rev-parse failed in {}: {}",
            workspace.display(),
            stderr
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        anyhow::bail!("git rev-parse returned an empty repository root");
    }
    Ok(PathBuf::from(stdout))
}

fn path_is_git_workspace(path: &Path) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_git_command(repo_root: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", repo_root.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            repo_root.display(),
            stderr
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn rewrite_local_dependency_paths_in_worktree(
    source_repo_root: &Path,
    worktree_root: &Path,
) -> Result<()> {
    let manifests = collect_worktree_manifests(worktree_root)?;
    for manifest_path in manifests {
        rewrite_manifest_local_paths(source_repo_root, worktree_root, &manifest_path)?;
    }
    Ok(())
}

fn collect_worktree_manifests(worktree_root: &Path) -> Result<Vec<PathBuf>> {
    let mut manifests = Vec::new();
    collect_worktree_manifests_recursive(worktree_root, &mut manifests)?;
    Ok(manifests)
}

fn collect_worktree_manifests_recursive(dir: &Path, manifests: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            if path.file_name().and_then(|name| name.to_str()) == Some(".git") {
                continue;
            }
            collect_worktree_manifests_recursive(&path, manifests)?;
        } else if file_type.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
        {
            manifests.push(path);
        }
    }
    Ok(())
}

fn rewrite_manifest_local_paths(
    source_repo_root: &Path,
    worktree_root: &Path,
    manifest_path: &Path,
) -> Result<()> {
    let source_manifest_path =
        source_manifest_path_for_worktree(source_repo_root, worktree_root, manifest_path);
    let source_manifest_dir = source_manifest_path
        .parent()
        .context("source manifest path missing parent directory")?;
    let raw = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let mut value: toml::Value = toml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", manifest_path.display()))?;
    let mut changed = false;
    rewrite_dependency_paths_in_table(&mut value, source_manifest_dir, &mut changed);
    if changed {
        let updated = toml::to_string_pretty(&value)
            .with_context(|| format!("failed to serialize {}", manifest_path.display()))?;
        fs::write(manifest_path, updated)
            .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    }
    Ok(())
}

fn source_manifest_path_for_worktree(
    source_repo_root: &Path,
    worktree_root: &Path,
    manifest_path: &Path,
) -> PathBuf {
    let relative = manifest_path
        .strip_prefix(worktree_root)
        .unwrap_or(manifest_path);
    source_repo_root.join(relative)
}

fn rewrite_dependency_paths_in_table(
    value: &mut toml::Value,
    source_manifest_dir: &Path,
    changed: &mut bool,
) {
    let Some(table) = value.as_table_mut() else {
        return;
    };

    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(section_value) = table.get_mut(section) {
            rewrite_named_dependency_section(section_value, source_manifest_dir, changed);
        }
    }

    if let Some(workspace) = table
        .get_mut("workspace")
        .and_then(toml::Value::as_table_mut)
    {
        if let Some(section_value) = workspace.get_mut("dependencies") {
            rewrite_named_dependency_section(section_value, source_manifest_dir, changed);
        }
    }

    if let Some(targets) = table.get_mut("target").and_then(toml::Value::as_table_mut) {
        for (_, target_value) in targets.iter_mut() {
            rewrite_dependency_paths_in_table(target_value, source_manifest_dir, changed);
        }
    }

    if let Some(patch) = table.get_mut("patch").and_then(toml::Value::as_table_mut) {
        for (_, registry_value) in patch.iter_mut() {
            rewrite_named_dependency_section(registry_value, source_manifest_dir, changed);
        }
    }
}

fn rewrite_named_dependency_section(
    section_value: &mut toml::Value,
    source_manifest_dir: &Path,
    changed: &mut bool,
) {
    let Some(section_table) = section_value.as_table_mut() else {
        return;
    };

    for (_, dependency) in section_table.iter_mut() {
        let Some(spec) = dependency.as_table_mut() else {
            continue;
        };
        let Some(path_value) = spec.get_mut("path") else {
            continue;
        };
        let Some(path_text) = path_value.as_str() else {
            continue;
        };
        let path = Path::new(path_text);
        if path.is_absolute() {
            continue;
        }
        let absolute = source_manifest_dir.join(path);
        let canonical = absolute.canonicalize().ok().unwrap_or(absolute);
        let absolute_text = canonical.to_string_lossy().into_owned();
        if absolute_text != path_text {
            *path_value = toml::Value::String(absolute_text);
            *changed = true;
        }
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
