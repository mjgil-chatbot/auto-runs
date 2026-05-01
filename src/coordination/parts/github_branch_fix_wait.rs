use super::*;

const GITHUB_BRANCH_FIX_WAITING_MARKER: &str = "WAITING_FOR_SUCCESS:";
const GITHUB_BRANCH_FIX_SUCCESS_MARKER: &str = "SUCCESS_CONFIRMED:";

pub fn github_branch_fix_poll_task_should_respawn(
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> bool {
    if run.workflow_mode != AutonomousWorkflowMode::GithubBranchFix
        || task.kind != CoordinationTaskKind::GeneratedReadTask
    {
        return false;
    }

    let title = task.title.to_ascii_lowercase();
    let prompt = task.prompt.to_ascii_lowercase();
    let acceptance_checks = task.acceptance_checks.join("\n").to_ascii_lowercase();
    title.contains("poll github actions")
        || title.contains("wait for github actions")
        || title.contains("verify newer github actions run completed successfully")
        || title.contains("validate github actions run completed successfully after fix push")
        || ((title.contains("poll") || title.contains("wait")) && prompt.contains("github actions"))
        || prompt.contains("do not report completion until a newer github actions run")
        || prompt.contains("completed successfully after the most recent fix push")
        || acceptance_checks.contains("latest run status is success")
        || acceptance_checks.contains("successful run identified or confirmed none exists post-fix")
}

pub fn github_branch_fix_poll_completion_should_respawn(
    run: &CoordinationRun,
    task: &CoordinationTask,
    artifact: &TaskArtifact,
) -> Result<bool> {
    if !github_branch_fix_poll_task_should_respawn(run, task) {
        return Ok(false);
    }
    if github_branch_fix_artifact_contains_marker(artifact, GITHUB_BRANCH_FIX_SUCCESS_MARKER) {
        return Ok(false);
    }
    match github_branch_fix_successful_run_for_head_exists(run, task) {
        Ok(true) => Ok(false),
        Ok(false) => Ok(true),
        Err(_) => Ok(github_branch_fix_artifact_contains_marker(
            artifact,
            GITHUB_BRANCH_FIX_WAITING_MARKER,
        )),
    }
}

pub fn requeue_github_branch_fix_poll_completion(
    run_id: &str,
    task: &mut CoordinationTask,
    provider_name: &str,
    artifact: &TaskArtifact,
) -> Result<bool> {
    let run = load_run(run_id)?;
    if run.status != CoordinationRunStatus::Active
        || !github_branch_fix_poll_completion_should_respawn(&run, task, artifact)?
    {
        return Ok(false);
    }

    let now = Utc::now();
    let current_attempt = task.attempt_count.max(1);
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.artifact_summary = Some(format!(
        "{} [{}]",
        artifact.summary,
        artifact.report.outcome.as_str()
    ));
    task.attempt_count = task.attempt_count.saturating_sub(1);
    task.updated_at = now;
    write_json_atomic(&task_path(run_id, &task.task_id), task)?;
    index_task(task);
    let _ = fs::remove_file(claim_path(run_id, &task.task_id));
    append_event_with_provider(
        run_id,
        "task_retry_scheduled",
        Some(&task.task_id),
        Some(provider_name),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Retry scheduled: {} after attempt {}/{}. GitHub Actions for the latest branch-fix commit has not succeeded yet, so the wait task returned to queue without consuming attempt budget. {}",
            task.title, current_attempt, task.max_attempts, artifact.summary
        ),
    )?;
    sync_run_summary_after_task_update(run_id, now)?;
    Ok(true)
}

pub fn github_branch_fix_wait_for_success_spec(run: &CoordinationRun) -> GeneratedTaskSpec {
    let config = github_branch_fix_config_from_run(run)
        .expect("GitHub branch fix config should exist for synthesized wait tasks");
    GeneratedTaskSpec {
        title: format!(
            "Wait for GitHub Actions {} run to complete successfully",
            config.branch
        ),
        provider: None,
        prompt: format!(
            "Poll GitHub Actions every {poll_minutes} minutes using `gh run list --branch {branch} --limit 5 --json databaseId,status,conclusion,headSha,createdAt` for the current repository. Also run `git rev-parse HEAD` so you can compare the latest local fix commit SHA against the run headSha values. Only mark this check complete once a run exists with `status=completed`, `conclusion=success`, and a `headSha` that matches the current `HEAD` or a newer fix commit pushed in this pass.\n\nIf no matching successful run exists yet, start the first line of your response with `{waiting_marker}` and include the latest observed run id, status, conclusion, and headSha so the runtime can retry this wait task later.\nIf a matching successful run exists, start the first line with `{success_marker}` and cite the successful run id and matching headSha.",
            branch = config.branch,
            poll_minutes = config.poll_minutes,
            waiting_marker = GITHUB_BRANCH_FIX_WAITING_MARKER,
            success_marker = GITHUB_BRANCH_FIX_SUCCESS_MARKER,
        ),
        kind: "read".to_string(),
        estimated_minutes: (config.poll_minutes.max(1) * 6).max(15),
        depends_on_titles: Vec::new(),
        read_scope: vec![
            ".github/workflows".to_string(),
            "scripts/parse-github-actions.sh".to_string(),
        ],
        write_scope: Vec::new(),
        allowed_tools: vec!["Bash".to_string(), "Grep".to_string(), "Glob".to_string()],
        acceptance_checks: vec![
            format!(
                "gh run list shows a completed run for `{}` with conclusion=success",
                config.branch
            ),
            "The successful run headSha matches the most recent fix commit pushed in this pass"
                .to_string(),
        ],
    }
}

fn github_branch_fix_artifact_contains_marker(artifact: &TaskArtifact, marker: &str) -> bool {
    artifact.body.contains(marker)
        || artifact.summary.contains(marker)
        || artifact.report.summary.contains(marker)
        || artifact.report.details.contains(marker)
}

fn github_branch_fix_successful_run_for_head_exists(
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> Result<bool> {
    #[derive(Deserialize)]
    struct GithubRunStatus {
        status: String,
        conclusion: Option<String>,
        #[serde(rename = "headSha")]
        head_sha: Option<String>,
    }

    let branch = github_branch_fix_config_from_run(run)
        .context("GitHub branch fix config should exist for poll completion checks")?
        .branch;
    let workspace = Path::new(&task.cwd);
    let head = github_branch_fix_wait_run_git(workspace, &["rev-parse", "HEAD"])?;
    let output = std::process::Command::new(github_branch_fix_gh_program())
        .arg("run")
        .arg("list")
        .arg("--branch")
        .arg(&branch)
        .arg("--limit")
        .arg("5")
        .arg("--json")
        .arg("status,conclusion,headSha")
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run gh run list in {}", workspace.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("gh run list failed in {}: {}", workspace.display(), stderr);
    }

    let runs: Vec<GithubRunStatus> = serde_json::from_slice(&output.stdout)
        .context("failed to parse gh run list output for branch-fix wait completion")?;
    Ok(runs.into_iter().any(|run| {
        run.status == "completed"
            && run.conclusion.as_deref() == Some("success")
            && run
                .head_sha
                .as_deref()
                .map(|head_sha| {
                    head_sha == head || head.starts_with(head_sha) || head_sha.starts_with(&head)
                })
                .unwrap_or(false)
    }))
}

fn github_branch_fix_wait_run_git(workspace: &Path, args: &[&str]) -> Result<String> {
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

fn github_branch_fix_gh_program() -> String {
    std::env::var("AUTO_RUNS_GH_BIN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "gh".to_string())
}

/// CI status for a branch-fix run — determines whether to run LLM tasks or stay silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GithubBranchFixCiStatus {
    /// CI run for the current HEAD is completed successfully.
    Success,
    /// CI run is still in progress (waiting for success).
    InProgress,
    /// CI run for the current HEAD has failed.
    Failed,
    /// Could not determine CI state (gh CLI error or no runs found).
    Unknown,
}

/// Returns the CI status for the branch in the given run.
/// Uses `gh run list` directly — no LLM involved.
///
/// - `Success`: a completed successful run exists for the current HEAD
/// - `InProgress`: runs exist but none has completed successfully for HEAD yet
/// - `Failed`: the latest run for HEAD has a non-success conclusion
/// - `Unknown`: no runs found or gh CLI failed
pub fn github_branch_fix_ci_status(run: &CoordinationRun) -> Result<GithubBranchFixCiStatus> {
    let config = github_branch_fix_config_from_run(run)
        .context("GitHub branch fix config should exist for CI status check")?;
    let workspace = Path::new(&run.workspace_path);
    let head = github_branch_fix_wait_run_git(workspace, &["rev-parse", "HEAD"])?;
    let output = std::process::Command::new(github_branch_fix_gh_program())
        .arg("run")
        .arg("list")
        .arg("--branch")
        .arg(&config.branch)
        .arg("--limit")
        .arg("5")
        .arg("--json")
        .arg("status,conclusion,headSha")
        .current_dir(workspace)
        .output()
        .with_context(|| format!("failed to run gh run list in {}", workspace.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::bail!("gh run list failed in {}: {}", workspace.display(), stderr);
    }

    #[derive(Deserialize)]
    struct GithubRunStatus {
        status: String,
        conclusion: Option<String>,
        #[serde(rename = "headSha")]
        head_sha: Option<String>,
    }

    let runs: Vec<GithubRunStatus> = serde_json::from_slice(&output.stdout)
        .context("failed to parse gh run list output for CI status check")?;

    // Find the run matching HEAD (same SHA prefix matching as the existing check)
    let head_match = runs.iter().find(|r| {
        r.head_sha
            .as_ref()
            .map(|hs| hs == &head || head.starts_with(hs) || hs.starts_with(&head))
            .unwrap_or(false)
    });

    if let Some(run) = head_match {
        if run.status == "completed" && run.conclusion.as_deref() == Some("success") {
            return Ok(GithubBranchFixCiStatus::Success);
        }
        if run.status == "in_progress" {
            return Ok(GithubBranchFixCiStatus::InProgress);
        }
        // completed but not success → failed
        return Ok(GithubBranchFixCiStatus::Failed);
    }

    // No runs found for HEAD — check if any run is in progress for this branch
    if runs.iter().any(|r| r.status == "in_progress") {
        return Ok(GithubBranchFixCiStatus::InProgress);
    }

    // No runs for this branch at all, or all completed without matching HEAD
    Ok(GithubBranchFixCiStatus::Unknown)
}
