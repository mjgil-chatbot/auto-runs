use super::*;

pub const GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY: &str = "github_branch_fix_branch";
pub const GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY: &str = "github_branch_fix_poll_minutes";
pub const GITHUB_BRANCH_FIX_SOURCE_WORKSPACE_NOTE_KEY: &str = "github_branch_fix_source_workspace";
pub const GITHUB_BRANCH_FIX_WORKTREE_PATH_NOTE_KEY: &str = "github_branch_fix_worktree_path";
const DEFAULT_GITHUB_BRANCH_FIX_POLL_MINUTES: u32 = 5;
const GITHUB_BRANCH_FIX_PLAN_TIMEOUT_MINUTES: u32 = 60;
const GITHUB_BRANCH_FIX_RUN_WORKTREE_SUFFIX_LEN: usize = 8;
const GITHUB_BRANCH_FIX_PUBLISH_RECOVERY_MARKER: &str = "Git publish recovery context:";
const GITHUB_BRANCH_FIX_GH_AUTH_REQUIREMENT: &str = "Before any `gh` Actions or API call, verify `gh auth status` succeeds. If your `gh` build supports `gh auth status --active`, that is preferred; otherwise fall back to plain `gh auth status`. Use existing `gh auth login` state or an authenticated `GH_TOKEN`/`GITHUB_TOKEN`; do not rely on anonymous GitHub API access. `gh run list` uses the GitHub REST API core rate limit, which is much lower when unauthenticated. If a `gh` call returns HTTP 403, check `gh auth status` and `gh api rate_limit` before continuing.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubBranchFixConfig {
    pub branch: String,
    pub poll_minutes: u32,
}

pub fn github_branch_fix_notes_with_settings(
    notes: &str,
    branch: &str,
    poll_minutes: u32,
) -> String {
    let updated = lightweight_runtime_notes_with_setting(
        notes,
        GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY,
        branch.trim(),
    );
    lightweight_runtime_notes_with_setting(
        &updated,
        GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY,
        &poll_minutes.max(1).to_string(),
    )
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

pub fn github_branch_fix_branch(run: &CoordinationRun) -> Option<String> {
    github_branch_fix_config_from_run(run).map(|config| config.branch)
}

pub fn github_branch_fix_poll_minutes(run: &CoordinationRun) -> Option<u32> {
    github_branch_fix_config_from_run(run).map(|config| config.poll_minutes)
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

pub fn github_branch_fix_publish_failure_should_respawn(
    run: &CoordinationRun,
    task: &CoordinationTask,
) -> bool {
    run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix
        && task.kind == CoordinationTaskKind::GeneratedWriteTask
        && task.attempt_count < task.max_attempts.max(1)
}

pub fn github_branch_fix_publish_failure_prompt(prompt: &str, failure_message: &str) -> String {
    let base = prompt
        .split_once(GITHUB_BRANCH_FIX_PUBLISH_RECOVERY_MARKER)
        .map(|(prefix, _)| prefix.trim_end())
        .unwrap_or_else(|| prompt.trim_end());
    format!(
        "{base}\n\n{marker}\n- The previous worker turn already wrote repo changes locally, but the runtime's automatic git commit or push failed.\n- Start from the current worktree state, inspect the git failure below, and make only the smallest additional changes needed so commit and push succeed.\n- Preserve existing staged or unstaged changes unless changing them is required to clear the publish blocker.\n- Rerun the relevant local checks before reporting completion.\n- If the intended local commit already exists, resolve the remaining push blocker without unnecessary code churn.\nLatest automatic git publish failure:\n{failure_message}",
        marker = GITHUB_BRANCH_FIX_PUBLISH_RECOVERY_MARKER,
    )
}

pub fn github_branch_fix_task_has_publish_recovery(task: &CoordinationTask) -> bool {
    task.prompt
        .contains(GITHUB_BRANCH_FIX_PUBLISH_RECOVERY_MARKER)
}

pub fn requeue_github_branch_fix_publish_failure(
    run_id: &str,
    task: &mut CoordinationTask,
    provider_name: &str,
    failure_message: &str,
) -> Result<bool> {
    let run = load_run(run_id)?;
    if !github_branch_fix_publish_failure_should_respawn(&run, task) {
        return Ok(false);
    }

    let now = Utc::now();
    let current_attempt = task.attempt_count.max(1);
    task.prompt = github_branch_fix_publish_failure_prompt(&task.prompt, failure_message);
    task.status = CoordinationTaskStatus::Queued;
    task.claimed_by = None;
    task.artifact_summary = Some(failure_message.to_string());
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
            "Retry scheduled: {} after automatic git publish failed on attempt {}/{}. Requeued the same branch-fix task so the worker can repair the publish blocker and finish the commit/push. {}",
            task.title, current_attempt, task.max_attempts, failure_message
        ),
    )?;
    sync_run_summary_after_task_update(run_id, now)?;
    Ok(true)
}

pub fn github_branch_fix_generated_task_prompt(run: &CoordinationRun, prompt: &str) -> String {
    let branch = github_branch_fix_config_from_run(run)
        .expect("GitHub branch fix config should exist for generated task prompts");
    format!(
        "This GitHub branch fix run targets branch `{}` with a {} minute poll cadence. Use the design doc at `{}` as the canonical local plan. Inspect real GitHub Actions failures directly before fixing them, preserve or expand relevant tests, and if you push a fix do not report completion until a newer GitHub Actions run for `{}` has completed successfully after that push.\n\n{}\n\n{}",
        branch.branch,
        branch.poll_minutes,
        run.design_doc_path,
        branch.branch,
        GITHUB_BRANCH_FIX_GH_AUTH_REQUIREMENT,
        prompt
    )
}

/// Tight fix-only plan prompt — used when CI has already failed and we need
/// the LLM to focus on parsing the failure and emitting targeted fix tasks.
/// Much shorter than the full bootstrap prompt to save tokens.
pub fn github_branch_fix_fix_only_plan_prompt(run: &CoordinationRun) -> String {
    let branch = github_branch_fix_config_from_run(run)
        .expect("GitHub branch fix config should exist for fix-only plan prompt");
    format!(
        "CI for branch `{}` has failed. Inspect the latest failed GitHub Actions run directly using `gh run view --repo <owner/repo> <run-id> --log` or similar. Fix the root cause, add or update tests, commit, and verify a newer successful GitHub Actions run for `{}` completed after the push.\n\n\
GitHub CLI auth: {}\n\n\
Return JSON only, no markdown fences. Each task needs title (= exact git commit message), kind (\"write\" or \"read\"), prompt, estimated_minutes, read_scope, write_scope.\n\
Rules: title = commit message, max 3 write_scope entries, prefer 3-8 narrow tasks.",
        branch.branch,
        branch.branch,
        GITHUB_BRANCH_FIX_GH_AUTH_REQUIREMENT
    )
}

pub fn validate_github_branch_fix_notes(notes: &str) -> Result<()> {
    let branch = lightweight_runtime_note_setting_value(notes, GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY)
        .map(str::trim)
        .unwrap_or_default();
    if branch.is_empty() {
        anyhow::bail!("github_branch_fix_branch is required for GitHub branch fix runs");
    }

    let poll_minutes =
        lightweight_runtime_note_setting_value(notes, GITHUB_BRANCH_FIX_POLL_MINUTES_NOTE_KEY)
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(DEFAULT_GITHUB_BRANCH_FIX_POLL_MINUTES);
    if poll_minutes == 0 {
        anyhow::bail!(
            "github_branch_fix_poll_minutes must be a positive integer number of minutes"
        );
    }
    Ok(())
}

pub fn validate_github_branch_fix_workspace_input(input: &UpsertRunInput) -> Result<()> {
    if input.workflow_mode != AutonomousWorkflowMode::GithubBranchFix {
        return Ok(());
    }

    let branch = github_branch_fix_input_branch(input)?;
    let workspace =
        inspect_github_branch_fix_workspace(Path::new(input.workspace_path.trim()), &branch)?;
    if let GithubBranchFixWorkspace::PrimaryCheckout { source_workspace } = workspace {
        let _ = github_branch_fix_target_worktree_path(
            &source_workspace,
            &branch,
            Some("preview-run"),
        )?;
    }
    Ok(())
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

pub fn seed_github_branch_fix_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    let branch = github_branch_fix_config_from_run(run)
        .expect("GitHub branch fix config should exist for GitHub branch fix runs");
    let worker_provider = run.provider.clone();
    let design_doc_exists = workspace_path_for_run(run, &run.design_doc_path).is_file();
    let survey_id = Uuid::new_v4().to_string();
    let implementation_plan_id = Uuid::new_v4().to_string();
    let validation_plan_id = Uuid::new_v4().to_string();
    let review_id = Uuid::new_v4().to_string();

    let mut tasks = vec![make_task(
        run,
        &survey_id,
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace".to_string(),
        format!(
            "Survey the workspace for context relevant to this GitHub branch fix goal: {}. \
The target branch is `{}` and the GitHub Actions poll cadence is every {} minute(s). \
The canonical design doc target is `{}`. \
If that document does not exist yet, note that the first implementation planning pass must bootstrap it from this goal and the workspace state.",
            goal, branch.branch, branch.poll_minutes, run.design_doc_path
        ),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Queued,
    )];

    let implementation_plan_prompt = format!(
        "Use this implementation idea as the starting point: {goal}.\n\n\
Target branch: `{branch}`.\n\
GitHub Actions poll interval: every {poll_minutes} minute(s).\n\
Canonical design doc path: `{design_doc}`.\n\n\
Bootstrap and execution requirements:\n\
- If `{design_doc}` does not exist yet, create it first from the goal and current workspace state.\n\
- The first write pass must ensure the branch has GitHub Actions coverage. Inspect `.github/workflows/`; if no workflow currently runs for `{branch}`, add or update a workflow so pushes to that branch start the repo's test workflow.\n\
- The first write pass must also ensure a versioned pre-commit hook exists for this repo and runs the repo test suite before commits. Prefer `.githooks/pre-commit` plus repo config over editing unrelated hook behavior.\n\
- After setup, emit worker tasks that poll GitHub Actions for `{branch}` every {poll_minutes} minute(s), inspect the latest failed run directly when one fails, fix the root cause in the branch, add or update tests for the fix, run the relevant local tests, commit, and push.\n\
- Any task that pushes fixes must verify that a newer GitHub Actions run for `{branch}` completed successfully after that push before the task reports completion.\n\n\
GitHub CLI authentication requirement:\n\
- {gh_auth_requirement}\n\n\
Return JSON only, with no markdown fences or commentary. The JSON must be an array of task objects.\n\n\
Rules:\n\
- Emit only implementation tasks for the current pass.\n\
- Keep tasks small and non-overlapping.\n\
- Use `kind: \"write\"` for tasks that modify the repo.\n\
- Use `kind: \"read\"` only when a worker must inspect GitHub state, logs, or local evidence before a later write.\n\
- For every write task, make `title` the exact git commit message the runtime should reuse verbatim after the task writes files.\n\
- Start `title` with an imperative subject line. If the task intentionally bundles multiple features, additional non-empty lines are allowed.\n\
- Every generated task must set a realistic integer `estimated_minutes` between 1 and 180.\n\
- If a fragment depends on another generated task, set `depends_on_titles` to the exact upstream task titles instead of relying on prose-only ordering notes.\n\
- Set `read_scope` to `[\"**\"]` for every generated task.\n\
- Set `write_scope` to `[\"**\"]` for every write task; read-only tasks should keep `write_scope` empty.\n\
- Patch-level git publish is the consolidation boundary, so do not try to narrow file ownership in task scopes.\n\
- Prefer 3-8 concrete fragments over broad umbrella tasks.\n\
- The final generated task in a fixing chain must not stop at `git push`; it must confirm a newer GitHub Actions run for `{branch}` completed successfully after the push.",
        goal = goal,
        branch = branch.branch,
        poll_minutes = branch.poll_minutes,
        design_doc = run.design_doc_path,
        gh_auth_requirement = GITHUB_BRANCH_FIX_GH_AUTH_REQUIREMENT,
    );
    let mut implementation_plan = make_task(
        run,
        &implementation_plan_id,
        Some(survey_id.clone()),
        CoordinationTaskKind::PlanFlatTasks,
        Some(worker_provider.clone()),
        "Split GitHub branch fix tasks".to_string(),
        implementation_plan_prompt,
        if run.allow_write_tasks {
            vec![
                "filesystem".to_string(),
                "search".to_string(),
                "edit".to_string(),
            ]
        } else {
            vec!["filesystem".to_string(), "search".to_string()]
        },
        vec!["**".to_string(), "artifacts/**".to_string()],
        if run.allow_write_tasks && !design_doc_exists {
            vec![run.design_doc_path.clone()]
        } else {
            Vec::new()
        },
        vec![survey_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    implementation_plan.task_category = Some(TaskCategory::Plan);
    implementation_plan.estimated_minutes = Some(GITHUB_BRANCH_FIX_PLAN_TIMEOUT_MINUTES);
    tasks.push(implementation_plan);

    let validation_plan_prompt = format!(
        "After the implementation fragments finish, break validation into small read-only tasks that confirm all of the following for branch `{branch}`:\n\
- the branch has GitHub Actions workflow coverage\n\
- the repo's pre-commit hook is present and configured to run tests\n\
- the latest fix work included relevant test coverage\n\
- a newer GitHub Actions run for `{branch}` completed successfully after the most recent fix push in this pass\n\n\
Return JSON only, with no markdown fences or commentary. The JSON must be an array of task objects.\n\n\
Rules:\n\
- Emit only validation tasks for the current pass.\n\
- Every validation task must use `kind: \"read\"`.\n\
- Every validation task must set a realistic integer `estimated_minutes` between 1 and 180.\n\
- Validation tasks must inspect the actual workspace state and GitHub run state, not just prior artifacts.\n\
- Keep validation tasks narrow and evidence-driven.\n\
- Do not emit write tasks from this planning step.",
        branch = branch.branch,
    );
    let mut validation_plan = make_task(
        run,
        &validation_plan_id,
        Some(implementation_plan_id.clone()),
        CoordinationTaskKind::PlanFlatTasks,
        Some(worker_provider.clone()),
        "Split GitHub branch fix validation tasks".to_string(),
        validation_plan_prompt,
        vec!["filesystem".to_string(), "search".to_string()],
        vec![
            "**".to_string(),
            "artifacts/**".to_string(),
            run.design_doc_path.clone(),
        ],
        Vec::new(),
        vec![implementation_plan_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    validation_plan.task_category = Some(TaskCategory::Verify);
    validation_plan.estimated_minutes = Some(GITHUB_BRANCH_FIX_PLAN_TIMEOUT_MINUTES);
    tasks.push(validation_plan);

    let mut review_task = make_task(
        run,
        &review_id,
        Some(validation_plan_id.clone()),
        CoordinationTaskKind::ReviewArtifacts,
        Some(worker_provider),
        "Validate GitHub branch fix pass".to_string(),
        github_branch_fix_review_prompt(run),
        vec!["filesystem".to_string(), "search".to_string()],
        vec![
            "artifacts/**".to_string(),
            "**".to_string(),
            run.design_doc_path.clone(),
        ],
        Vec::new(),
        vec![validation_plan_id],
        CoordinationTaskStatus::Blocked,
    );
    review_task.task_category = Some(TaskCategory::Review);
    tasks.push(review_task);

    tasks
}

pub fn github_branch_fix_review_prompt(run: &CoordinationRun) -> String {
    let branch = github_branch_fix_config_from_run(run)
        .expect("GitHub branch fix config should exist for GitHub branch fix review prompts");
    format!(
        "Validate the completed branch-fix work against the design doc `{design_doc}` and the current GitHub Actions state for branch `{branch}`.\n\n\
Complete the pass only when the evidence shows all of the following:\n\
- the branch has GitHub Actions workflow coverage\n\
- the repo has a pre-commit hook that runs tests before commits\n\
- the latest fix added or preserved relevant test coverage\n\
- a newer GitHub Actions run for `{branch}` completed successfully after the most recent fix push made in this pass\n\n\
GitHub CLI authentication requirement:\n\
- {gh_auth_requirement}\n\n\
If more work remains:\n\
- explain the remaining gaps\n\
- emit smaller follow-up fix tasks so the branch-fix worker can continue from the current workspace state\n\n\
If the implementation is complete:\n\
- emit no fix tasks\n\
- mark the work as complete only after explicitly citing the observed GitHub Actions run that completed successfully for `{branch}` after the most recent fix push.",
        design_doc = run.design_doc_path,
        branch = branch.branch,
        gh_auth_requirement = GITHUB_BRANCH_FIX_GH_AUTH_REQUIREMENT,
    )
}

fn github_branch_fix_input_branch(input: &UpsertRunInput) -> Result<String> {
    lightweight_runtime_note_setting_value(&input.notes, GITHUB_BRANCH_FIX_BRANCH_NOTE_KEY)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .context("github_branch_fix_branch is required for GitHub branch fix runs")
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
