use super::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
/// Rust project scaffold root. Override with the `AUTO_RUNS_RUST_SCAFFOLD_ROOT` environment variable.
pub(super) fn rust_scaffold_root() -> String {
    std::env::var("AUTO_RUNS_RUST_SCAFFOLD_ROOT").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_default()
            .join("git")
            .join("rust-project-scaffold")
            .to_string_lossy()
            .into_owned()
    })
}
/// Zig project scaffold root. Override with the `AUTO_RUNS_ZIG_SCAFFOLD_ROOT` environment variable.
pub(super) fn zig_scaffold_root() -> String {
    std::env::var("AUTO_RUNS_ZIG_SCAFFOLD_ROOT").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_default()
            .join("git")
            .join("zig-project-scaffold")
            .to_string_lossy()
            .into_owned()
    })
}
const RUST_PROJECT_SCAFFOLD_EXCLUDED_ROOTS: [&str; 2] = [".git", "target"];
const LIGHTWEIGHT_BUILD_ARTIFACT_DIRS: [&str; 3] = ["target", ".zig-cache", "zig-out"];
const COMMON_LIGHTWEIGHT_GITIGNORE_ENTRIES: [&str; 1] = ["target"];
const ZIG_PROJECT_SCAFFOLD_FILES: [(&str, bool); 3] = [
    ("build.zig", false),
    ("build.zig.zon", false),
    ("lib/compiler.zig", false),
];
const ZIG_GITIGNORE_CONTENT: &str = "/.zig-cache/\n/zig-out/\n";
const ZIG_PRE_COMMIT_HOOK_CONTENT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"

if zig fmt --check . --exclude .git --exclude .zig-cache --exclude zig-out 2>&1; then
    echo "pre-commit: fmt check passed"
else
    echo "pre-commit: zig fmt check failed"
    exit 1
fi

if zig build test 2>&1; then
    echo "pre-commit: tests passed"
else
    echo "pre-commit: tests failed"
    exit 1
fi
"#;
const GIT_HOOK_INSTALLER_CONTENT: &str = r#"#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
git -C "$repo_root" config core.hooksPath .githooks

echo "Configured Git hooks path to .githooks"
"#;
pub fn ensure_task_workspace_exists(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
) -> Result<bool> {
    let workspace = Path::new(task.cwd.trim());
    if workspace.is_dir() {
        if uses_lightweight_runtime_mode(&run.workflow_mode) {
            ensure_lightweight_workspace_ready(run, task, provider, workspace)?;
        }
        return Ok(false);
    }

    if run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix {
        let mut refreshed_run = run.clone();
        if ensure_github_branch_fix_workspace_ready(&mut refreshed_run)? && workspace.is_dir() {
            append_event_with_provider(
                &run.run_id,
                "task_workspace_created",
                Some(&task.task_id),
                provider,
                task.task_category,
                task.provider_hint.as_deref(),
                &format!(
                    "Recreated GitHub branch fix worktree at {} before running {}.",
                    workspace.display(),
                    task.title
                ),
            )?;
            return Ok(true);
        }
        anyhow::bail!(
            "GitHub branch fix worktree {} is missing and could not be recreated",
            workspace.display()
        );
    }

    fs::create_dir_all(workspace).with_context(|| {
        format!(
            "failed to create missing workspace directory {} for run {} task {}",
            workspace.display(),
            run.run_id,
            task.task_id
        )
    })?;

    append_event_with_provider(
        &run.run_id,
        "task_workspace_created",
        Some(&task.task_id),
        provider,
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Created missing workspace directory at {} before running {}.",
            workspace.display(),
            task.title
        ),
    )?;

    if uses_lightweight_runtime_mode(&run.workflow_mode) {
        ensure_lightweight_workspace_ready(run, task, provider, workspace)?;
    }

    Ok(true)
}
fn ensure_lightweight_workspace_ready(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
    workspace: &Path,
) -> Result<()> {
    let initialized_repo = if workspace_is_git_repo(workspace) {
        false
    } else {
        run_git_command(workspace, &["init", "-b", "main"])?;
        true
    };
    ensure_git_identity_configured(workspace)?;
    if initialized_repo && !super::lightweight_publish::git_head_exists(workspace)? {
        run_git_command(
            workspace,
            &[
                "commit",
                "--allow-empty",
                "-m",
                "Initialize autonomous workspace",
            ],
        )?;
    }

    let github_remote = ensure_lightweight_origin_remote(run, task, provider, workspace)?;
    if github_remote
        .as_ref()
        .is_some_and(|remote| remote.uses_isolated_branch)
    {
        let branch = ensure_isolated_lightweight_branch(workspace)?;
        append_event_with_provider(
            &run.run_id,
            "task_workspace_git_branch_selected",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Selected isolated lightweight branch `{branch}` for existing GitHub repository before running {}.",
                task.title
            ),
        )?;
    }
    maybe_configure_lightweight_repo(run, task, provider, workspace, initialized_repo)?;
    ensure_lightweight_gitignore_hygiene(run, task, provider, workspace)?;
    repair_lightweight_build_artifact_permissions(run, task, provider, workspace)?;

    if initialized_repo {
        append_event_with_provider(
            &run.run_id,
            "task_workspace_git_initialized",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Initialized lightweight git repository at {}{} before running {}.",
                workspace.display(),
                github_remote
                    .as_ref()
                    .map(|repo| format!(" with GitHub origin {}", repo.repo_name))
                    .unwrap_or_default(),
                task.title
            ),
        )?;
    }

    Ok(())
}
fn ensure_lightweight_gitignore_hygiene(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
    workspace: &Path,
) -> Result<()> {
    let mut required_entries = COMMON_LIGHTWEIGHT_GITIGNORE_ENTRIES.to_vec();
    if lightweight_repo_configuration_for_run(run) == LightweightRepoConfiguration::Zig {
        required_entries.extend([".zig-cache", "zig-out"]);
    }

    let added_entries =
        append_missing_gitignore_entries(&workspace.join(".gitignore"), &required_entries)?;
    if !added_entries.is_empty() {
        append_event_with_provider(
            &run.run_id,
            "task_workspace_gitignore_updated",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Updated .gitignore before running {} to ignore: {}.",
                task.title,
                added_entries.join(", ")
            ),
        )?;
    }

    Ok(())
}
fn repair_lightweight_build_artifact_permissions(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
    workspace: &Path,
) -> Result<()> {
    let mut repaired_dirs = Vec::new();
    for dir_name in LIGHTWEIGHT_BUILD_ARTIFACT_DIRS {
        let artifact_root = workspace.join(dir_name);
        if !artifact_root.exists() {
            continue;
        }
        repair_path_user_permissions(&artifact_root)?;
        repaired_dirs.push(dir_name.to_string());
    }

    if !repaired_dirs.is_empty() {
        append_event_with_provider(
            &run.run_id,
            "task_workspace_permissions_repaired",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Repaired build artifact permissions before running {} for: {}.",
                task.title,
                repaired_dirs.join(", ")
            ),
        )?;
    }

    Ok(())
}
fn maybe_configure_lightweight_repo(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
    workspace: &Path,
    initialized_repo: bool,
) -> Result<()> {
    match lightweight_repo_configuration_for_run(run) {
        LightweightRepoConfiguration::None => Ok(()),
        LightweightRepoConfiguration::Rust => {
            if !initialized_repo && !workspace_has_only_docs_directory(workspace)? {
                return Ok(());
            }
            apply_rust_project_scaffold_configuration(workspace)?;
            append_event_with_provider(
                &run.run_id,
                "task_workspace_repo_configured",
                Some(&task.task_id),
                provider,
                task.task_category,
                task.provider_hint.as_deref(),
                &format!(
                    "Applied Rust repo configuration from {} before running {}.",
                    rust_scaffold_root(),
                    task.title
                ),
            )?;
            Ok(())
        }
        LightweightRepoConfiguration::Zig => {
            apply_zig_project_configuration(workspace)?;
            append_event_with_provider(
                &run.run_id,
                "task_workspace_repo_configured",
                Some(&task.task_id),
                provider,
                task.task_category,
                task.provider_hint.as_deref(),
                &format!(
                    "Applied Zig repo configuration from {} before running {}.",
                    zig_scaffold_root(),
                    task.title
                ),
            )?;
            Ok(())
        }
    }
}
fn apply_rust_project_scaffold_configuration(workspace: &Path) -> Result<()> {
    let scaffold_root_str = rust_scaffold_root();
    let scaffold_root = Path::new(&scaffold_root_str);
    if !scaffold_root.is_dir() {
        anyhow::bail!(
            "Rust scaffold root {} is unavailable. Set AUTO_RUNS_RUST_SCAFFOLD_ROOT to configure.",
            scaffold_root_str
        );
    }

    copy_repo_configuration_tree(scaffold_root, workspace)?;
    configure_git_hooks_path(workspace)?;
    Ok(())
}
fn apply_zig_project_configuration(workspace: &Path) -> Result<()> {
    let scaffold_root_str = zig_scaffold_root();
    let scaffold_root = Path::new(&scaffold_root_str);
    if !scaffold_root.is_dir() {
        anyhow::bail!(
            "Zig scaffold root {} is unavailable. Set AUTO_RUNS_ZIG_SCAFFOLD_ROOT to configure.",
            scaffold_root_str
        );
    }

    for (relative_path, executable) in ZIG_PROJECT_SCAFFOLD_FILES {
        let source = scaffold_root.join(relative_path);
        let destination = workspace.join(relative_path);
        copy_repo_configuration_file(&source, &destination, executable)?;
    }

    write_repo_configuration_contents(&workspace.join(".gitignore"), ZIG_GITIGNORE_CONTENT, false)?;
    write_repo_configuration_contents(
        &workspace.join(".githooks/pre-commit"),
        ZIG_PRE_COMMIT_HOOK_CONTENT,
        true,
    )?;
    write_repo_configuration_contents(
        &workspace.join("scripts/install-git-hooks.sh"),
        GIT_HOOK_INSTALLER_CONTENT,
        true,
    )?;
    run_command(workspace, "./scripts/install-git-hooks.sh", &[])?;
    Ok(())
}
fn workspace_has_only_docs_directory(workspace: &Path) -> Result<bool> {
    let mut visible_entries = Vec::new();
    for entry in fs::read_dir(workspace)
        .with_context(|| format!("failed to read {}", workspace.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", workspace.display()))?;
        let file_name = entry.file_name();
        if file_name.to_string_lossy() == ".git" {
            continue;
        }
        visible_entries.push(entry);
        if visible_entries.len() > 1 {
            return Ok(false);
        }
    }

    let Some(entry) = visible_entries.into_iter().next() else {
        return Ok(false);
    };
    let is_docs = entry.file_name().to_string_lossy() == "docs";
    let is_dir = entry
        .file_type()
        .with_context(|| format!("failed to inspect {}", entry.path().display()))?
        .is_dir();
    Ok(is_docs && is_dir)
}
fn copy_repo_configuration_file(source: &Path, destination: &Path, executable: bool) -> Result<()> {
    if !source.is_file() {
        anyhow::bail!("repo configuration source {} is missing", source.display());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to copy repo configuration file from {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    if executable {
        ensure_executable(destination)?;
    }
    Ok(())
}
fn copy_repo_configuration_tree(source_root: &Path, destination_root: &Path) -> Result<()> {
    for entry in fs::read_dir(source_root)
        .with_context(|| format!("failed to read {}", source_root.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", source_root.display()))?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if RUST_PROJECT_SCAFFOLD_EXCLUDED_ROOTS.contains(&file_name.as_ref()) {
            continue;
        }
        copy_repo_configuration_path(&entry.path(), &destination_root.join(file_name.as_ref()))?;
    }
    Ok(())
}
fn copy_repo_configuration_path(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to read {}", source.display()))?;
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .with_context(|| format!("failed to create {}", destination.display()))?;
        for entry in
            fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", source.display()))?;
            let file_name = entry.file_name();
            copy_repo_configuration_path(&entry.path(), &destination.join(file_name))?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        anyhow::bail!("unsupported scaffold entry {}", source.display());
    }

    let executable = metadata.permissions().mode() & 0o111 != 0;
    copy_repo_configuration_file(source, destination, executable)
}
fn write_repo_configuration_contents(
    destination: &Path,
    contents: &str,
    executable: bool,
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(destination, contents)
        .with_context(|| format!("failed to write {}", destination.display()))?;
    if executable {
        ensure_executable(destination)?;
    }
    Ok(())
}
fn append_missing_gitignore_entries(destination: &Path, entries: &[&str]) -> Result<Vec<String>> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let mut contents = if destination.is_file() {
        fs::read_to_string(destination)
            .with_context(|| format!("failed to read {}", destination.display()))?
    } else {
        String::new()
    };
    let existing = contents
        .lines()
        .map(normalize_gitignore_entry_for_compare)
        .collect::<std::collections::HashSet<_>>();

    let mut added = Vec::new();
    for entry in entries {
        let normalized = normalize_gitignore_entry_for_compare(entry);
        if normalized.is_empty() || existing.contains(&normalized) {
            continue;
        }
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(entry.trim());
        contents.push('\n');
        added.push(entry.trim().to_string());
    }

    if !added.is_empty() {
        fs::write(destination, contents)
            .with_context(|| format!("failed to write {}", destination.display()))?;
    }

    Ok(added)
}
fn normalize_gitignore_entry_for_compare(entry: &str) -> String {
    entry
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/')
        .trim_end_matches('/')
        .to_string()
}
fn configure_git_hooks_path(workspace: &Path) -> Result<()> {
    run_git_command(workspace, &["config", "core.hooksPath", ".githooks"])
}
fn ensure_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)
            .with_context(|| format!("failed to read {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).with_context(|| {
            format!("failed to set executable permissions on {}", path.display())
        })?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
fn repair_path_user_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let mut permissions = metadata.permissions();
        let mode = permissions.mode();
        let updated_mode = if metadata.is_dir() {
            mode | 0o700
        } else if mode & 0o111 != 0 {
            mode | 0o700
        } else {
            mode | 0o600
        };
        if updated_mode != mode {
            permissions.set_mode(updated_mode);
            fs::set_permissions(path, permissions)
                .with_context(|| format!("failed to repair permissions on {}", path.display()))?;
        }

        if metadata.is_dir() {
            for entry in
                fs::read_dir(path).with_context(|| format!("failed to read {}", path.display()))?
            {
                let entry = entry.with_context(|| format!("failed to read {}", path.display()))?;
                repair_path_user_permissions(&entry.path())?;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }

    Ok(())
}
fn ensure_git_identity_configured(workspace: &Path) -> Result<()> {
    if git_config_value(workspace, "user.name")?.is_some()
        && git_config_value(workspace, "user.email")?.is_some()
    {
        return Ok(());
    }

    let user_name = preferred_git_user_name();
    let user_email = preferred_git_user_email(&user_name);
    run_git_command(workspace, &["config", "user.name", user_name.as_str()])?;
    run_git_command(workspace, &["config", "user.email", user_email.as_str()])?;
    Ok(())
}
fn preferred_git_user_name() -> String {
    for key in ["GIT_AUTHOR_NAME", "GIT_COMMITTER_NAME", "USER", "USERNAME"] {
        if let Some(value) = std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return value;
        }
    }
    "local-user".to_string()
}
fn preferred_git_user_email(user_name: &str) -> String {
    for key in ["GIT_AUTHOR_EMAIL", "GIT_COMMITTER_EMAIL", "EMAIL"] {
        if let Some(value) = std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            return value;
        }
    }

    let host = std::env::var("HOSTNAME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "local".to_string());
    format!("{user_name}@{host}.local")
}
#[derive(Clone, Debug)]
struct GithubRemoteBootstrap {
    repo_name: String,
    uses_isolated_branch: bool,
}
#[derive(Clone, Debug)]
struct ExistingGithubRemote {
    repo_name: String,
    remote_url: String,
}
fn ensure_lightweight_origin_remote(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider: Option<&str>,
    workspace: &Path,
) -> Result<Option<GithubRemoteBootstrap>> {
    if let Some(origin_url) = git_remote_url(workspace, "origin")? {
        if let Some(repo_name) = github_repo_name_from_remote_url(&origin_url) {
            remove_remote_if_present(workspace, "github")?;
            return Ok(Some(GithubRemoteBootstrap {
                repo_name,
                uses_isolated_branch: false,
            }));
        }
    }

    if let Some(existing_remote) = existing_github_remote_for_workspace(workspace)? {
        set_remote_url(workspace, "origin", &existing_remote.remote_url)?;
        remove_remote_if_present(workspace, "github")?;
        append_event_with_provider(
            &run.run_id,
            "task_workspace_github_remote_created",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Attached GitHub remote `origin` for {} from existing repository {}.",
                task.title, existing_remote.repo_name
            ),
        )?;
        return Ok(Some(GithubRemoteBootstrap {
            repo_name: existing_remote.repo_name,
            uses_isolated_branch: true,
        }));
    }

    let Some(gh_program) = github_cli_program() else {
        append_event_with_provider(
            &run.run_id,
            "task_workspace_github_remote_skipped",
            Some(&task.task_id),
            provider,
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Skipped GitHub origin bootstrap for {} because the gh CLI is unavailable.",
                task.title
            ),
        )?;
        return Ok(None);
    };

    let repo_name = github_repo_name_for_workspace(workspace);
    match gh_repo_create(workspace, &gh_program, &repo_name, "origin") {
        Ok(()) => {
            remove_remote_if_present(workspace, "github")?;
            append_event_with_provider(
                &run.run_id,
                "task_workspace_github_remote_created",
                Some(&task.task_id),
                provider,
                task.task_category,
                task.provider_hint.as_deref(),
                &format!(
                    "Created or attached GitHub remote `origin` for {} as repository {}.",
                    task.title, repo_name
                ),
            )?;
            Ok(Some(GithubRemoteBootstrap {
                repo_name,
                uses_isolated_branch: false,
            }))
        }
        Err(create_err) => {
            match gh_repo_attach_existing(workspace, &gh_program, &repo_name, "origin") {
                Ok(()) => {
                    remove_remote_if_present(workspace, "github")?;
                    append_event_with_provider(
                        &run.run_id,
                        "task_workspace_github_remote_created",
                        Some(&task.task_id),
                        provider,
                        task.task_category,
                        task.provider_hint.as_deref(),
                        &format!(
                            "Attached existing GitHub remote `origin` for {} as repository {}.",
                            task.title, repo_name
                        ),
                    )?;
                    Ok(Some(GithubRemoteBootstrap {
                        repo_name,
                        uses_isolated_branch: true,
                    }))
                }
                Err(attach_err) => {
                    append_event_with_provider(
                &run.run_id,
                "task_workspace_github_remote_skipped",
                Some(&task.task_id),
                provider,
                task.task_category,
                task.provider_hint.as_deref(),
                &format!(
                    "Skipped GitHub origin bootstrap for {}: create failed with `{}` and attach failed with `{}`",
                    task.title, create_err, attach_err
                ),
            )?;
                    Ok(None)
                }
            }
        }
    }
}
fn github_cli_program() -> Option<String> {
    let program = std::env::var("AUTO_RUNS_GH_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gh".to_string());
    command_available(&program).then_some(program)
}
fn command_available(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
fn github_repo_name_for_workspace(workspace: &Path) -> String {
    let base = workspace
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("lightweight-workspace");
    let mut sanitized = base
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => ch,
            _ => '-',
        })
        .collect::<String>();
    sanitized = sanitized.trim_matches('-').to_string();
    if sanitized.is_empty() {
        "lightweight-workspace".to_string()
    } else {
        sanitized
    }
}
fn existing_github_remote_for_workspace(workspace: &Path) -> Result<Option<ExistingGithubRemote>> {
    for remote_name in git_remote_names(workspace)? {
        if remote_name == "github" {
            continue;
        }
        let Some(remote_url) = git_remote_url(workspace, &remote_name)? else {
            continue;
        };
        let Some(repo_name) = github_repo_name_from_remote_url(&remote_url) else {
            continue;
        };
        return Ok(Some(ExistingGithubRemote {
            repo_name,
            remote_url,
        }));
    }
    Ok(None)
}
fn github_repo_name_from_remote_url(remote_url: &str) -> Option<String> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    let path = if let Some(rest) = trimmed.strip_prefix("git@github.com:") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("ssh://git@github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("http://github.com/") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("git://github.com/") {
        rest
    } else {
        return None;
    };
    let repo = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = repo.split('/');
    let _owner = segments.next()?;
    let name = segments.next()?;
    if name.is_empty() || segments.next().is_some() {
        return None;
    }
    Some(name.to_string())
}
fn gh_repo_create(
    workspace: &Path,
    gh_program: &str,
    repo_name: &str,
    remote_name: &str,
) -> Result<()> {
    run_command(
        workspace,
        gh_program,
        &[
            "repo",
            "create",
            repo_name,
            "--source=.",
            &format!("--remote={remote_name}"),
            "--private",
        ],
    )
}
fn gh_repo_attach_existing(
    workspace: &Path,
    gh_program: &str,
    repo_name: &str,
    remote_name: &str,
) -> Result<()> {
    let remote_url = command_stdout(
        workspace,
        gh_program,
        &[
            "repo", "view", repo_name, "--json", "sshUrl", "--jq", ".sshUrl",
        ],
    )?;
    set_remote_url(workspace, remote_name, &remote_url)
}
fn set_remote_url(workspace: &Path, remote_name: &str, remote_url: &str) -> Result<()> {
    if git_remote_url(workspace, remote_name)?.is_some() {
        run_git_command(workspace, &["remote", "set-url", remote_name, remote_url])
    } else {
        run_git_command(workspace, &["remote", "add", remote_name, remote_url])
    }
}
fn remove_remote_if_present(workspace: &Path, remote_name: &str) -> Result<()> {
    if git_remote_url(workspace, remote_name)?.is_some() {
        run_git_command(workspace, &["remote", "remove", remote_name])?;
    }
    Ok(())
}
fn run_git_command(root: &Path, args: &[&str]) -> Result<()> {
    run_command(root, "git", args)
}
fn run_command(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| {
            format!(
                "failed to run {} {} in {}",
                program,
                args.join(" "),
                root.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "{} {} failed in {}: {}",
            program,
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
fn command_stdout(root: &Path, program: &str, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .with_context(|| {
            format!(
                "failed to run {} {} in {}",
                program,
                args.join(" "),
                root.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "{} {} failed in {}: {}",
            program,
            args.join(" "),
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn git_remote_names(root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["remote"])
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to list git remotes in {}", root.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "git remote failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}
fn git_config_value(root: &Path, key: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", key])
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to read git config {} in {}", key, root.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}
fn git_remote_url(root: &Path, remote_name: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", remote_name])
        .current_dir(root)
        .output()
        .with_context(|| {
            format!(
                "failed to read git remote {} in {}",
                remote_name,
                root.display()
            )
        })?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}
fn ensure_isolated_lightweight_branch(workspace: &Path) -> Result<String> {
    if let Some(current_branch) = git_current_branch(workspace)? {
        if current_branch.starts_with("auto-runs-") {
            return Ok(current_branch);
        }
    }

    let branch = next_auto_runs_branch_name(workspace)?;
    run_git_command(workspace, &["checkout", "--orphan", branch.as_str()])?;
    Ok(branch)
}
fn git_current_branch(root: &Path) -> Result<Option<String>> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .with_context(|| format!("failed to inspect current branch in {}", root.display()))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}
fn next_auto_runs_branch_name(workspace: &Path) -> Result<String> {
    let mut max_suffix = 0usize;

    let local_output = std::process::Command::new("git")
        .args(["branch", "--list", "auto-runs-*"])
        .current_dir(workspace)
        .output()
        .with_context(|| {
            format!(
                "failed to list local auto-runs branches in {}",
                workspace.display()
            )
        })?;
    if local_output.status.success() {
        for line in String::from_utf8_lossy(&local_output.stdout).lines() {
            update_auto_runs_branch_suffix_max(
                line.trim().trim_start_matches('*').trim(),
                &mut max_suffix,
            );
        }
    }

    let remote_output = std::process::Command::new("git")
        .args(["ls-remote", "--heads", "origin", "auto-runs-*"])
        .current_dir(workspace)
        .output()
        .with_context(|| {
            format!(
                "failed to list remote auto-runs branches in {}",
                workspace.display()
            )
        })?;
    if remote_output.status.success() {
        for line in String::from_utf8_lossy(&remote_output.stdout).lines() {
            if let Some(branch) = line.split_whitespace().nth(1) {
                update_auto_runs_branch_suffix_max(
                    branch.trim_start_matches("refs/heads/"),
                    &mut max_suffix,
                );
            }
        }
    }

    Ok(format!("auto-runs-{}", max_suffix + 1))
}
fn update_auto_runs_branch_suffix_max(branch: &str, max_suffix: &mut usize) {
    let Some(suffix) = branch.strip_prefix("auto-runs-") else {
        return;
    };
    if let Ok(value) = suffix.parse::<usize>() {
        *max_suffix = (*max_suffix).max(value);
    }
}
fn workspace_is_git_repo(root: &Path) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(root)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}
