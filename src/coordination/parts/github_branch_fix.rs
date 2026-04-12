use super::*;
pub const GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY: &str = "github_branch_fix_branch";
pub const GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY: &str = "github_branch_fix_poll_minutes";
pub const GITHUB_BRANCH_FIX_SOURCE_WORKSPACE_NOTE_KEY: &str = "github_branch_fix_source_workspace";
pub const GITHUB_BRANCH_FIX_WORKTREE_PATH_NOTE_KEY: &str = "github_branch_fix_worktree_path";
const DEFAULT_GITHUB_BRANCH_FIX_POLL_MINUTES: u32 = 5;
const GITHUB_BRANCH_FIX_RUN_WORKTREE_SUFFIX_LEN: usize = 8;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubBranchFixConfig {
    pub branch: String,
    pub poll_minutes: u32,
}
pub fn github_branch_fix_config_from_run(run: &CoordinationRun) -> Option<GithubBranchFixConfig> {
    (run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix).then(|| GithubBranchFixConfig {
        branch: lightweight_runtime_note_setting_value(
            &run.notes,
            GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY,
        )
        .unwrap_or_default()
        .to_string(),
        poll_minutes: lightweight_runtime_note_setting_value(
            &run.notes,
            GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY,
        )
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_GITHUB_BRANCH_FIX_POLL_MINUTES),
    })
}
pub fn github_branch_fix_source_workspace(run: &CoordinationRun) -> Option<String> {
    (run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix).then(|| {
        lightweight_runtime_note_setting_value(
            &run.notes,
            GITHUB_BRANCH_FIX_SOURCE_WORKSPACE_NOTE_KEY,
        )
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(run.workspace_path.as_str())
        .to_string()
    })
}
pub fn github_branch_fix_worktree_path(run: &CoordinationRun) -> Option<String> {
    (run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix).then(|| {
        lightweight_runtime_note_setting_value(&run.notes, GITHUB_BRANCH_FIX_WORKTREE_PATH_NOTE_KEY)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(run.workspace_path.as_str())
            .to_string()
    })
}
pub fn prepare_github_branch_fix_workspace(run: &mut CoordinationRun) -> Result<()> {
    if run.workflow_mode != AutonomousWorkflowMode::GithubBranchFix {
        return Ok(());
    }

    let config = github_branch_fix_config_from_run(run)
        .context("GitHub branch fix config should exist before preparing worktree")?;
    let requested_workspace = PathBuf::from(run.workspace_path.trim());
    let workspace_selection =
        inspect_github_branch_fix_workspace(&requested_workspace, &config.branch)
            .or_else(|_| github_branch_fix_workspace_from_notes(run, &config.branch))?;

    let (source_workspace, worktree_path) = match workspace_selection {
        GithubBranchFixWorkspace::PrimaryCheckout { source_workspace } => {
            let worktree_path = github_branch_fix_target_worktree_path(
                &source_workspace,
                &config.branch,
                Some(run.run_id.as_str()),
            )?;
            if worktree_path.exists() {
                if github_branch_fix_worktree_matches(&source_workspace, &worktree_path)? {
                    checkout_github_branch_fix_worktree_branch(
                        &source_workspace,
                        &worktree_path,
                        &config.branch,
                    )?;
                } else {
                    anyhow::bail!(
                        "GitHub branch fix worktree path {} already exists but is not a reusable worktree for source workspace {}",
                        worktree_path.display(),
                        source_workspace.display()
                    );
                }
            } else {
                create_github_branch_fix_worktree(
                    &source_workspace,
                    &worktree_path,
                    &config.branch,
                )?;
            }
            (source_workspace, worktree_path)
        }
        GithubBranchFixWorkspace::ExistingWorktree {
            source_workspace,
            worktree_path,
        } => (source_workspace, worktree_path),
    };

    let worktree_path_text = worktree_path.to_string_lossy().into_owned();
    run.workspace_path = worktree_path_text.clone();
    run.notes = lightweight_runtime_notes_with_setting(
        &run.notes,
        GITHUB_BRANCH_FIX_SOURCE_WORKSPACE_NOTE_KEY,
        &source_workspace.to_string_lossy(),
    );
    run.notes = lightweight_runtime_notes_with_setting(
        &run.notes,
        GITHUB_BRANCH_FIX_WORKTREE_PATH_NOTE_KEY,
        &worktree_path_text,
    );
    Ok(())
}
pub fn ensure_github_branch_fix_workspace_ready(run: &mut CoordinationRun) -> Result<bool> {
    if run.workflow_mode != AutonomousWorkflowMode::GithubBranchFix {
        return Ok(false);
    }

    let Some(worktree_path) = github_branch_fix_worktree_path(run) else {
        prepare_github_branch_fix_workspace(run)?;
        return Ok(true);
    };
    if Path::new(&worktree_path).is_dir() {
        return Ok(false);
    }

    prepare_github_branch_fix_workspace(run)?;
    Ok(true)
}
fn github_branch_fix_source_workspace_path(run: &CoordinationRun) -> Result<PathBuf> {
    let source = github_branch_fix_source_workspace(run)
        .context("GitHub branch fix source workspace should exist before preparing worktree")?;
    canonical_primary_github_branch_fix_workspace(&source)
}
fn canonical_primary_github_branch_fix_workspace(raw: &str) -> Result<PathBuf> {
    match inspect_github_branch_fix_workspace(Path::new(raw.trim()), "main")? {
        GithubBranchFixWorkspace::PrimaryCheckout { source_workspace } => Ok(source_workspace),
        GithubBranchFixWorkspace::ExistingWorktree { .. } => {
            anyhow::bail!("expected a primary GitHub branch fix workspace root")
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum GithubBranchFixWorkspace {
    PrimaryCheckout {
        source_workspace: PathBuf,
    },
    ExistingWorktree {
        source_workspace: PathBuf,
        worktree_path: PathBuf,
    },
}
fn inspect_github_branch_fix_workspace(
    workspace: &Path,
    branch: &str,
) -> Result<GithubBranchFixWorkspace> {
    let canonical_workspace = workspace
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace_path {}", workspace.display()))?;
    let repo_root = github_branch_fix_git_repo_root(&canonical_workspace)?;
    if canonical_workspace != repo_root {
        anyhow::bail!(
            "GithubBranchFix workspace_path {} must point at the git repository root, not a subdirectory",
            canonical_workspace.display()
        );
    }

    let common_dir = github_branch_fix_git_common_dir(&canonical_workspace)?;
    let source_workspace = common_dir
        .parent()
        .context("GitHub branch fix common git dir is missing its repository parent")?
        .canonicalize()
        .with_context(|| {
            format!(
                "failed to resolve primary checkout root for {}",
                canonical_workspace.display()
            )
        })?;

    if source_workspace == canonical_workspace {
        return Ok(GithubBranchFixWorkspace::PrimaryCheckout { source_workspace });
    }

    let current_branch = github_branch_fix_current_branch(&canonical_workspace)?;
    if current_branch != branch {
        anyhow::bail!(
            "GithubBranchFix workspace_path {} is an existing linked worktree on branch {}, but the run requested branch {}",
            canonical_workspace.display(),
            current_branch,
            branch
        );
    }

    Ok(GithubBranchFixWorkspace::ExistingWorktree {
        source_workspace,
        worktree_path: canonical_workspace,
    })
}
fn github_branch_fix_workspace_from_notes(
    run: &CoordinationRun,
    branch: &str,
) -> Result<GithubBranchFixWorkspace> {
    let source_workspace = github_branch_fix_source_workspace_path(run)?;
    let Some(worktree_path) = github_branch_fix_worktree_path(run) else {
        anyhow::bail!("GitHub branch fix workspace notes are missing a worktree path");
    };
    let worktree_path = PathBuf::from(worktree_path.trim());
    if worktree_path == source_workspace {
        return Ok(GithubBranchFixWorkspace::PrimaryCheckout { source_workspace });
    }
    Ok(GithubBranchFixWorkspace::ExistingWorktree {
        source_workspace: source_workspace.clone(),
        worktree_path: if worktree_path.is_absolute() {
            worktree_path
        } else {
            github_branch_fix_target_worktree_path(
                &source_workspace,
                branch,
                Some(run.run_id.as_str()),
            )?
        },
    })
}
fn github_branch_fix_target_worktree_path(
    source_workspace: &Path,
    branch: &str,
    run_id: Option<&str>,
) -> Result<PathBuf> {
    let suffix = sanitize_github_branch_fix_branch_for_path(branch);
    let run_suffix = github_branch_fix_run_worktree_suffix(run_id);
    let source_text = source_workspace
        .to_str()
        .context("GitHub branch fix source workspace path must be valid UTF-8")?;
    Ok(PathBuf::from(format!(
        "{source_text}-{suffix}-{run_suffix}"
    )))
}
fn sanitize_github_branch_fix_branch_for_path(branch: &str) -> String {
    let mut result = String::new();
    let mut last_was_separator = false;

    for ch in branch.trim().chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            result.push(ch);
            last_was_separator = false;
            continue;
        }
        if !last_was_separator && !result.is_empty() {
            result.push('-');
        }
        last_was_separator = true;
    }

    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "branch".to_string()
    } else {
        trimmed.to_string()
    }
}
fn github_branch_fix_run_worktree_suffix(run_id: Option<&str>) -> String {
    let raw = run_id.unwrap_or("run");
    let mut suffix = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(GITHUB_BRANCH_FIX_RUN_WORKTREE_SUFFIX_LEN)
        .collect::<String>()
        .to_ascii_lowercase();
    if suffix.is_empty() {
        suffix = "run".to_string();
    }
    suffix
}
fn create_github_branch_fix_worktree(
    source_workspace: &Path,
    worktree_path: &Path,
    branch: &str,
) -> Result<()> {
    let parent = worktree_path
        .parent()
        .context("GitHub branch fix worktree path missing parent directory")?;
    fs::create_dir_all(parent)?;
    let worktree_text = worktree_path.to_string_lossy().into_owned();

    if github_branch_fix_local_branch_exists(source_workspace, branch) {
        github_branch_fix_run_git(
            source_workspace,
            &["worktree", "add", "--force", &worktree_text, branch],
        )?;
        return Ok(());
    }

    if github_branch_fix_remote_branch_exists(source_workspace, branch) {
        let remote_ref = format!("origin/{branch}");
        github_branch_fix_run_git(
            source_workspace,
            &[
                "worktree",
                "add",
                "--force",
                "-b",
                branch,
                &worktree_text,
                &remote_ref,
            ],
        )?;
        return Ok(());
    }

    github_branch_fix_run_git(
        source_workspace,
        &[
            "worktree",
            "add",
            "--force",
            "-b",
            branch,
            &worktree_text,
            "HEAD",
        ],
    )?;
    Ok(())
}
fn checkout_github_branch_fix_worktree_branch(
    source_workspace: &Path,
    worktree_path: &Path,
    branch: &str,
) -> Result<()> {
    let current_branch = github_branch_fix_current_branch(worktree_path)?;
    if current_branch == branch {
        return Ok(());
    }

    if github_branch_fix_local_branch_exists(source_workspace, branch) {
        github_branch_fix_run_git(worktree_path, &["checkout", branch])?;
        return Ok(());
    }

    if github_branch_fix_remote_branch_exists(source_workspace, branch) {
        let remote_ref = format!("origin/{branch}");
        github_branch_fix_run_git(worktree_path, &["checkout", "-b", branch, &remote_ref])?;
        return Ok(());
    }

    github_branch_fix_run_git(worktree_path, &["checkout", "-B", branch, "HEAD"])?;
    Ok(())
}
fn github_branch_fix_worktree_matches(
    source_workspace: &Path,
    worktree_path: &Path,
) -> Result<bool> {
    if !worktree_path.is_dir() {
        return Ok(false);
    }
    let worktree_common_dir = match github_branch_fix_git_common_dir(worktree_path) {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    let source_common_dir = github_branch_fix_git_common_dir(source_workspace)?;
    Ok(worktree_common_dir == source_common_dir)
}
fn github_branch_fix_local_branch_exists(source_workspace: &Path, branch: &str) -> bool {
    github_branch_fix_git_status(
        source_workspace,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
}
fn github_branch_fix_remote_branch_exists(source_workspace: &Path, branch: &str) -> bool {
    github_branch_fix_git_status(
        source_workspace,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/remotes/origin/{branch}"),
        ],
    )
}
fn github_branch_fix_current_branch(worktree_path: &Path) -> Result<String> {
    github_branch_fix_run_git(worktree_path, &["rev-parse", "--abbrev-ref", "HEAD"])
}
fn github_branch_fix_git_common_dir(workspace: &Path) -> Result<PathBuf> {
    let common_dir = github_branch_fix_run_git(workspace, &["rev-parse", "--git-common-dir"])?;
    let common_dir_path = Path::new(common_dir.trim());
    let absolute = if common_dir_path.is_absolute() {
        common_dir_path.to_path_buf()
    } else {
        workspace.join(common_dir_path)
    };
    absolute.canonicalize().with_context(|| {
        format!(
            "failed to resolve git common dir for {}",
            workspace.display()
        )
    })
}
fn github_branch_fix_git_repo_root(workspace: &Path) -> Result<PathBuf> {
    let root = github_branch_fix_run_git(workspace, &["rev-parse", "--show-toplevel"])?;
    Ok(PathBuf::from(root))
}
fn github_branch_fix_run_git(workspace: &Path, args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", workspace.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!(
            "git {} failed in {}: {}",
            args.join(" "),
            workspace.display(),
            stderr
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
fn github_branch_fix_git_status(workspace: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}
