use super::*;

/// Returns the validator script path appropriate for the current execution environment.
/// In runsc mode, the script is baked into the container image at a known path.
fn task_report_validator_path() -> String {
    match std::env::var("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV")
        .ok()
        .as_deref()
    {
        Some("runsc") => "/usr/local/bin/validate_autonomous_task_report.py".to_string(),
        _ => TASK_REPORT_VALIDATOR.to_string(),
    }
}

pub fn task_report_validator_hint(task: &CoordinationTask) -> Option<String> {
    if task.kind == CoordinationTaskKind::PlanFlatTasks && lookup_contract_for_task(task).is_none()
    {
        return None;
    }
    let run = load_run(&task.run_id).ok();
    if run.as_ref().is_some_and(|run| {
        uses_lightweight_runtime_mode(&run.workflow_mode)
            && matches!(
                task.kind,
                CoordinationTaskKind::GeneratedReadTask | CoordinationTaskKind::GeneratedWriteTask
            )
    }) {
        return None;
    }
    let omit_changed_files = run
        .as_ref()
        .is_some_and(|run| lightweight_task_omits_changed_files(run, task));
    let (kind, include_workspace_root) = match task.kind {
        CoordinationTaskKind::ReviewArtifacts => ("review", true),
        CoordinationTaskKind::DraftDesignDoc
        | CoordinationTaskKind::ConvergeDesignDoc
        | CoordinationTaskKind::MaterializeDesignDoc
        | CoordinationTaskKind::GeneratedReadTask
        | CoordinationTaskKind::GeneratedWriteTask
        | CoordinationTaskKind::ApplyApprovedEdits
        | CoordinationTaskKind::RalphPlan
        | CoordinationTaskKind::RalphExecute => ("standard", true),
        CoordinationTaskKind::PlanFlatTasks => ("plan", omit_changed_files),
        CoordinationTaskKind::RalphVerify => ("ralph_verify", false),
        CoordinationTaskKind::PlanCodeReview | CoordinationTaskKind::SurveyWorkspace => {
            return None;
        }
        CoordinationTaskKind::PerfBaseline
        | CoordinationTaskKind::PerfParse
        | CoordinationTaskKind::PerfBench
        | CoordinationTaskKind::PerfEvaluate => {
            return None;
        }
        CoordinationTaskKind::PerfApply => ("standard", true),
    };

    let workspace_flag = if include_workspace_root {
        format!(
            " --workspace-root {}",
            provider_visible_workspace_root(task)
        )
    } else {
        String::new()
    };
    let changed_files_flag = if omit_changed_files {
        " --allow-missing-changed-files"
    } else {
        ""
    };
    Some(format!(
        "Before you answer, validate your candidate JSON with `python3 {} --kind {}{}{} /tmp/task-report.json`. If it fails, fix the output and validate again before returning it.",
        task_report_validator_path(), kind, workspace_flag, changed_files_flag
    ))
}

pub fn provider_visible_workspace_root(task: &CoordinationTask) -> &str {
    match std::env::var("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV")
        .ok()
        .as_deref()
    {
        Some("runsc") => ".",
        _ => task.cwd.as_str(),
    }
}

fn typed_envelope_contract_intro(
    task_kind: CoordinationTaskKind,
    include_changed_files: bool,
) -> String {
    let contract = lookup_contract(task_kind).expect("typed contract must exist");
    let fields = if include_changed_files {
        "`outcome`, `summary`, `details`, `changed_files`, `tests_run`, `warnings`, `blockers`, `remaining_gaps`, `design_requirements_satisfied`, `validation_errors`, `payload_kind`, `contract_version`, and `payload`."
    } else {
        "`outcome`, `summary`, `details`, `tests_run`, `warnings`, `blockers`, `remaining_gaps`, `design_requirements_satisfied`, `validation_errors`, `payload_kind`, `contract_version`, and `payload`."
    };
    format!(
        "Return JSON only, with no markdown fences or commentary. The top-level JSON object must be a typed task artifact envelope with these fields: \
{}\n\n\
Envelope rules:\n\
- `payload_kind` must be `{}`.\n\
- `contract_version` must be {}.\n\
- `validation_errors` must be `[]` when your response is valid.\n",
        fields,
        contract.payload_kind.as_str(),
        contract.contract_version
    )
}

pub fn provider_response_contract(task: &CoordinationTask) -> String {
    let review_metadata = load_review_task_metadata(&task.run_id, &task.task_id);
    let run = load_run(&task.run_id).ok();
    let validator_hint = task_report_validator_hint(task)
        .map(|hint| format!("\n\n{}", hint))
        .unwrap_or_default();
    match task.kind {
        CoordinationTaskKind::PlanFlatTasks
            if run
                .as_ref()
                .map(|run| run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix)
                .unwrap_or(false) => format!(
            "Return JSON only, with no markdown fences or commentary. The top-level response must be a JSON array of task objects.\n\n\
Array rules:\n\
- Emit a non-empty array.\n\
- The first non-whitespace response character must be `[` and the last non-whitespace character must be `]`; do not add prose, labels, or markdown fences before or after the array.\n\
- Each task object must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `kind` must be either `read` or `write`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same plan. Use `[]` when the task has no generated-task prerequisites.\n\
- All `read_scope` and `write_scope` entries must be repo-relative workspace paths, never absolute paths such as `/home/...`.\n\
- Do not include a `provider` field; provider assignment is handled by the coordination engine.\n\
- Set `read_scope` to `[\"**\"]` for every generated task.\n\
- Set `write_scope` to `[\"**\"]` for every write task; read-only tasks should keep `write_scope` empty.\n\
- Patch-level git publish is the consolidation boundary for write tasks.\n\
- Each write task must include at least 2 concrete `acceptance_checks` strings.{}",
            validator_hint
        ),
        CoordinationTaskKind::PlanFlatTasks
            if run
                .as_ref()
                .map(|run| uses_lightweight_runtime_mode(&run.workflow_mode))
                .unwrap_or(false)
                && task_mutates_workspace(task) => format!(
            "{}\n\
Payload rules:\n\
- Put the generated task list in `payload.generated_tasks`.\n\
- `payload.generated_tasks` must be a non-empty array of task objects.\n\
- Each generated task must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same plan. Use `[]` when the task has no generated-task prerequisites.\n\
- All `read_scope` and `write_scope` entries must be repo-relative workspace paths, never absolute paths such as `/home/...`.\n\
- Do not include a `provider` field unless you are intentionally overriding provider assignment.\n\
- Generated tasks should inspect the full repo: set `read_scope` to `[\"**\"]`.\n\
- Generated write tasks should also set `write_scope` to `[\"**\"]`; patch-level git publish is the consolidation boundary.\n\
- Generated read tasks should keep `write_scope` empty.\n\
- If you bootstrap or refresh workspace documents inside the declared write scope, set `outcome` to `completed_with_changes`.\n\
- If you only planned tasks without editing the workspace, use `completed_no_change`.\n\
- Do not include a `changed_files` field; the coordinator derives workspace write effects from the declared write scope and resulting filesystem state.\n\
- Use `details` for concise planning rationale, not for the task list itself.{}",
            typed_envelope_contract_intro(task.kind, false),
            validator_hint
        ),
        CoordinationTaskKind::PlanFlatTasks => format!(
            "{}\n\
Payload rules:\n\
- Put the generated task list in `payload.generated_tasks`.\n\
- `payload.generated_tasks` must be a non-empty array of task objects.\n\
- Each generated task must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same plan. Use `[]` when the task has no generated-task prerequisites.\n\
- All `read_scope` and `write_scope` entries must be repo-relative workspace paths, never absolute paths such as `/home/...`.\n\
- Do not include a `provider` field unless you are intentionally overriding provider assignment.\n\
- Set `read_scope` to `[\"**\"]` for every generated task.\n\
- Set `write_scope` to `[\"**\"]` for every write task; read-only tasks should keep `write_scope` empty.\n\
- Patch-level git publish is the consolidation boundary for write tasks.\n\
- `changed_files` must be `[]`.\n\
- Use `details` for concise planning rationale, not for the task list itself.{}",
            typed_envelope_contract_intro(task.kind, true),
            validator_hint
        ),
        CoordinationTaskKind::RalphVerify => format!(
        "Return JSON only, with no markdown fences. \
        The JSON object must contain `complete` (boolean) and `reason` (string). \
        Set `complete` to true only if every criterion is satisfied right now. \
        If `complete` is false, `reason` must explain exactly what remains.\n\n\
        Example: {{\"complete\": false, \"reason\": \"Tests pass but file upload is not implemented yet.\"}}{:}",
        validator_hint
    ),
        CoordinationTaskKind::RalphPlan => format!(
            "Return JSON only, with no markdown fences or commentary. The JSON object must contain: \
`outcome`, `summary`, `details`, `changed_files`, `tests_run`, `warnings`, `blockers`, \
`remaining_gaps`, and `design_requirements_satisfied`.\n\n\
Rules:\n\n\
- Put the full Ralph working plan in `details` as a JSON string containing markdown only.\n\n\
- The markdown in `details` must include sections titled `Raw Acceptance Criteria`, `Status Board`, `Current Focus`, and `Evidence`.\n\n\
- `Raw Acceptance Criteria` must carry forward every completion criterion from the run configuration.\n\n\
- `Status Board` must mark each criterion as `not_started`, `partial`, `complete`, or `blocked`.\n\n\
- `Current Focus` must list only the smallest next actions for criteria that are not complete.\n\n\
- `changed_files` must be `[]` because this task updates the plan artifact, not the workspace.\n\n\
- Use `partial` when criteria remain incomplete, `completed_no_change` when all criteria are currently satisfied, and `blocked` only when there is no viable next step.\n\n\
- Explicitly list which criteria/status rows were refreshed in `design_requirements_satisfied`.\n\n\
- Acceptance checks for this task:\n{}{}",
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
        CoordinationTaskKind::ReviewArtifacts if review_metadata.is_some() => format!(
            "{}\n\
Payload rules:\n\
- Put structured findings in `payload.findings` and any follow-up tasks in `payload.fix_tasks`.\n\
- `payload.findings` must be an array of objects. Each finding must contain `severity`, `confidence`, `file`, `line`, `dimension`, `description`, and `fix_suggestion`.\n\
- `severity` must be one of `high`, `medium`, or `low`.\n\
- Every finding must cite the current workspace with a concrete `file:line` reference.\n\
- `changed_files` must be `[]` because this is a read-only review task.\n\
- Keep the strongest evidence in `payload.findings`; use `details` for the supporting narrative and review context.\n\
- In report-only review mode, `payload.fix_tasks` must be `[]`.\n\
- If you emit write-kind `payload.fix_tasks`, keep each one to one concrete capability but still use repo-wide scopes so patch-level git publish can reconcile the result.\n\
- Every `payload.fix_tasks` item should set `read_scope` to `[\"**\"]`.\n\
- Every write-kind `payload.fix_tasks` item should set `write_scope` to `[\"**\"]`; read-only fix tasks should keep `write_scope` empty.\n\
- Each write-kind `payload.fix_tasks` item must include at least 2 concrete `acceptance_checks` strings.{}",
            typed_envelope_contract_intro(task.kind, true),
            validator_hint
        ),
        CoordinationTaskKind::ReviewArtifacts
            if run
                .as_ref()
                .map(|run| run.workflow_mode == AutonomousWorkflowMode::GithubBranchFix)
                .unwrap_or(false) => format!(
            "{}\n\
Payload rules:\n\
- Put structured findings in `payload.findings` and the next branch-fix tasks in `payload.fix_tasks`.\n\
- `payload.findings` may be empty only when the branch is healthy and the pass is complete.\n\
- If more work remains, use `details` to summarize the remaining branch, workflow, test, or GitHub-run gaps and emit smaller `payload.fix_tasks`.\n\
- If the implementation is complete, emit no `payload.fix_tasks`.\n\
- Do not include a `changed_files` field; this is a read-only review task.\n\
- Every item in `payload.fix_tasks` must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same review pass. Use `[]` when the task has no generated-task prerequisites.\n\
- Do not include a `provider` field in `payload.fix_tasks`; provider assignment is handled by the coordination engine.\n\
- Make `payload.fix_tasks` smaller than the current pass: one concrete capability per write task, but keep repo-wide scopes so patch-level git publish can reconcile the result.\n\
- Every `payload.fix_tasks` item should set `read_scope` to `[\"**\"]`.\n\
- Every write-kind `payload.fix_tasks` item should set `write_scope` to `[\"**\"]`; read-only fix tasks should keep `write_scope` empty.\n\
- Each write-kind fix task must include at least 2 concrete `acceptance_checks` strings.\n\
- Use `details` for the validation rationale and concrete GitHub Actions evidence, not raw markdown outside the JSON object.{}",
            typed_envelope_contract_intro(task.kind, false),
            validator_hint
        ),
        CoordinationTaskKind::ReviewArtifacts
            if run
                .as_ref()
                .map(|run| uses_lightweight_runtime_mode(&run.workflow_mode))
                .unwrap_or(false) => format!(
            "{}\n\
Payload rules:\n\
- Put structured findings in `payload.findings` and the next implementation tasks in `payload.fix_tasks`.\n\
- `payload.findings` may be empty only when the implementation is complete.\n\
- If more work remains, do a full reassessment of the repo state against the completed current checklist.\n\
- If that reassessment changes the plan materially, update the canonical design doc inside the declared write scope before writing the next task-list document.\n\
- Write the next task-list document to the declared write scope for the next pass.\n\
- If the implementation is complete, emit no `payload.fix_tasks`.\n\
- Do not include a `changed_files` field; the coordinator derives design-doc and next-task-list writes from the declared write scope and the resulting filesystem state.\n\
- Every item in `payload.fix_tasks` must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same review pass. Use `[]` when the task has no generated-task prerequisites.\n\
- Do not include a `provider` field in `payload.fix_tasks`; provider assignment is handled by the coordination engine.\n\
- Make `payload.fix_tasks` smaller than the current pass: one concrete capability per write task, but keep repo-wide scopes so patch-level git publish can reconcile the result.\n\
- Every `payload.fix_tasks` item should set `read_scope` to `[\"**\"]`.\n\
- Every write-kind `payload.fix_tasks` item should set `write_scope` to `[\"**\"]`; read-only fix tasks should keep `write_scope` empty.\n\
- Each write-kind fix task must include at least 2 concrete `acceptance_checks` strings.\n\
- Use `details` for the next-pass task list summary and the validation rationale, not raw markdown outside the JSON object.{}",
            typed_envelope_contract_intro(task.kind, false),
            validator_hint
        ),
        CoordinationTaskKind::ReviewArtifacts => format!(
            "{}\n\
Payload rules:\n\
- Put structured findings in `payload.findings` and any follow-up tasks in `payload.fix_tasks`.\n\
- `payload.findings` may be empty only when the repo is already compliant.\n\
- `details` must explain the review findings against the current workspace state.\n\
- If `blockers` or `remaining_gaps` are non-empty, you must include `payload.fix_tasks` as a JSON array.\n\
- Each item in `payload.fix_tasks` must contain `title`, `prompt`, `kind`, `estimated_minutes`, `depends_on_titles`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\
- `estimated_minutes` must be a realistic integer minute estimate between 1 and 180. Tasks that exceed that estimate are timed out and retried.\n\
- Use `depends_on_titles` as an array of exact upstream generated task titles from the same review pass. Use `[]` when the task has no generated-task prerequisites.\n\
- Do not include a `provider` field in `payload.fix_tasks`; provider assignment is handled by the coordination engine.\n\
- Make `payload.fix_tasks` smaller than the current generated tasks: one concrete capability per write task, but keep repo-wide scopes so patch-level git publish can reconcile the result.\n\
- Every `payload.fix_tasks` item should set `read_scope` to `[\"**\"]`.\n\
- Every write-kind `payload.fix_tasks` item should set `write_scope` to `[\"**\"]`; read-only fix tasks should keep `write_scope` empty.\n\
- Each write-kind fix task must include at least 2 concrete `acceptance_checks` strings (e.g. \"cargo check succeeds\", \"function X is called in module Y\").\n\
- `changed_files` must be `[]` because this is a read-only review task.{}",
            typed_envelope_contract_intro(task.kind, true),
            validator_hint
        ),
        CoordinationTaskKind::DraftDesignDoc => format!(
            "{}\n\
Payload rules:\n\n\
- Put the full markdown design proposal in `payload.markdown`. Do not output raw markdown outside the JSON object.\n\n\
- `summary` must describe the proposal effect, not just that you ran.\n\n\
- `details` must briefly explain the proposal and the main tradeoffs.\n\n\
- `changed_files` must be `[]` for this task because the proposal belongs in the artifact, not the workspace.\n\n\
- If you are blocked, use `outcome: \"blocked\"` and populate `blockers`.\n\n\
- If work is incomplete, use `outcome: \"partial\"` and populate `remaining_gaps`.\n\n\
- Explicitly list which requirements you satisfied in `design_requirements_satisfied`.\n\n\
- Acceptance checks for this task:\n{}{}",
            typed_envelope_contract_intro(task.kind, true),
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
        CoordinationTaskKind::ConvergeDesignDoc => format!(
            "{}\n\
Payload rules:\n\n\
- Put the full canonical markdown design doc in `payload.markdown`. Do not output raw markdown outside the JSON object.\n\n\
- `summary` must describe the synthesis outcome, not just that you ran.\n\n\
- `details` must briefly explain the synthesis decision and any unresolved tradeoffs.\n\n\
- `changed_files` must be `[]` for this task because the converged document belongs in the artifact, not the workspace.\n\n\
- If you are blocked, use `outcome: \"blocked\"` and populate `blockers`.\n\n\
- If work is incomplete, use `outcome: \"partial\"` and populate `remaining_gaps`.\n\n\
- Explicitly list which requirements you satisfied in `design_requirements_satisfied`.\n\n\
- Acceptance checks for this task:\n{}{}",
            typed_envelope_contract_intro(task.kind, true),
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
        CoordinationTaskKind::GeneratedWriteTask
            if run
                .as_ref()
                .map(|run| uses_lightweight_runtime_mode(&run.workflow_mode))
                .unwrap_or(false) => format!(
            "Return concise plain text only. Do not return a task-report JSON object.\n\n\
Start the first line with exactly one status prefix:\n\
- `COMPLETED:` when you finished the slice and changed the workspace\n\
- `NO_CHANGE:` when the slice was already satisfied and you left the workspace as-is\n\
- `PARTIAL:` when you made some progress but follow-up work remains\n\
- `BLOCKED:` when you could not proceed because of a real blocker\n\
- `FAILED:` when the attempt failed unexpectedly\n\n\
After the status line, briefly summarize what you changed or verified, mention any noteworthy follow-up context, and explicitly call out which acceptance checks were satisfied. Do not spend tokens tracking file lists.\n\n\
Acceptance checks for this task:\n{}",
            format_lines(&task.acceptance_checks),
        ),
        CoordinationTaskKind::GeneratedReadTask
            if run
                .as_ref()
                .map(|run| uses_lightweight_runtime_mode(&run.workflow_mode))
                .unwrap_or(false) => format!(
            "Return concise plain text only. Do not return a task-report JSON object.\n\n\
Start the first line with exactly one status prefix:\n\
- `COMPLETED:` when you finished the validation slice\n\
- `NO_CHANGE:` when there was nothing new to report beyond confirming current state\n\
- `PARTIAL:` when you gathered some evidence but more validation work remains\n\
- `BLOCKED:` when a real blocker prevented validation\n\
- `FAILED:` when the attempt failed unexpectedly\n\n\
After the status line, summarize the concrete evidence you gathered from the workspace and explicitly call out which acceptance checks were satisfied.\n\n\
Acceptance checks for this task:\n{}",
            format_lines(&task.acceptance_checks),
        ),
        CoordinationTaskKind::GeneratedWriteTask => format!(
            "Return JSON only, with no markdown fences or commentary. The JSON object must contain: \
`outcome`, `summary`, `details`, `changed_files`, `tests_run`, `warnings`, `blockers`, \
`remaining_gaps`, and `design_requirements_satisfied`.\n\n\
Rules:\n\n\
- `outcome` must be one of `completed_with_changes`, `completed_no_change`, `blocked`, `partial`, or `failed`.\n\n\
- `summary` must describe the write effect, not just that you ran.\n\n\
- `details` must contain the full final contents of every changed document/text file, not a prose summary.\n\n\
- If you changed one primary markdown artifact, put the complete final file contents directly in `details`.\n\n\
- If you changed multiple files, include each final file body in `details` with clear file labels.\n\n\
- `changed_files` must list the workspace-relative file paths you changed. Use `[]` only if you changed nothing.\n\n\
- If you are blocked, use `outcome: \"blocked\"` and populate `blockers`.\n\n\
- If work is incomplete, use `outcome: \"partial\"` and populate `remaining_gaps`.\n\n\
- Explicitly list which requirements you satisfied in `design_requirements_satisfied`.\n\n\
- Acceptance checks for this task:\n{}{}",
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
        CoordinationTaskKind::MaterializeDesignDoc => format!(
            "Return JSON only, with no markdown fences or commentary. The JSON object must contain: \
`outcome`, `summary`, `details`, `changed_files`, `tests_run`, `warnings`, `blockers`, \
`remaining_gaps`, and `design_requirements_satisfied`.\n\n\
Rules:\n\n\
- `outcome` must be one of `completed_with_changes`, `completed_no_change`, `blocked`, `partial`, or `failed`.\n\n\
- `details` must explain exactly what file content you wrote and where, and it must include the full final document text rather than a summary.\n\n\
- `summary` must describe the write effect, not just that you ran.\n\n\
- `changed_files` must list the workspace-relative file paths you changed. Use `[]` only if you changed nothing.\n\n\
- If you are blocked, use `outcome: \"blocked\"` and populate `blockers`.\n\n\
- If work is incomplete, use `outcome: \"partial\"` and populate `remaining_gaps`.\n\n\
- Explicitly list which requirements you satisfied in `design_requirements_satisfied`.\n\n\
- Acceptance checks for this task:\n{}{}",
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
        CoordinationTaskKind::PerfBaseline | CoordinationTaskKind::PerfBench => {
            format!(
                "Return ONLY a JSON object — no markdown fences, no commentary, no extra text. \
Use the standardized perf benchmark artifact schema below. \
Include `correctness.status` as `pass`, `fail`, or `unknown`; when the benchmark output is legacy timing-only text, \
normalize it into this schema and use `unknown` with an explanatory summary. \
Keep the legacy flat timing fields for compatibility. \
If the build failed, set `build_failed: true`, include `error`, leave `metrics.cases` empty, and still return valid JSON.\n\n{}",
                perf_benchmark_contract_text(task.kind == CoordinationTaskKind::PerfBench)
            )
        }
        CoordinationTaskKind::PerfParse => {
            format!(
                "{}\n\
Payload rules:\n\
- Put the parsed ideas in `payload.ideas`.\n\
- Each idea must contain `index` (integer, 0-based), `title` (string ≤60 chars), and `description` (string with the full change detail).\n\
- Order ideas largest expected speedup first.\n\
- `changed_files` must be `[]`.\n\
- Use `details` for a short rationale about why the ideas were prioritized this way.",
                typed_envelope_contract_intro(task.kind, true)
            )
        }
        CoordinationTaskKind::PerfEvaluate => {
            "Return ONLY a JSON object — no markdown fences, no commentary. \
The object must contain: `keep` (boolean), `idea_index` (integer), \
`reason` (string, one sentence with measured numbers), `improvement_pct` (float). \
Positive improvement_pct means faster. Negative means slower.".to_string()
        }
        _ => format!(
            "Return JSON only, with no markdown fences or commentary. The JSON object must contain: \
`outcome`, `summary`, `details`, `changed_files`, `tests_run`, `warnings`, `blockers`, \
`remaining_gaps`, and `design_requirements_satisfied`.\n\n\
Rules:\n\
- `outcome` must be one of `completed_with_changes`, `completed_no_change`, `blocked`, `partial`, or `failed`.\n\
- `summary` must describe the effect, not just that you ran.\n\
- `details` must contain the substantive artifact text or explanation.\n\
- `changed_files` must list workspace-relative file paths you changed. Use `[]` if you changed nothing.\n\
- If you are blocked, use `outcome: \"blocked\"` and populate `blockers`.\n\
- If work is incomplete, use `outcome: \"partial\"` and populate `remaining_gaps`.\n\
- When this task has acceptance checks, explicitly cover them in `design_requirements_satisfied`.\n\
- Acceptance checks for this task:\n{}{}",
            format_lines(&task.acceptance_checks),
            validator_hint
        ),
    }
}
