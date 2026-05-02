pub fn load_run(run_id: &str) -> Result<CoordinationRun> {
    let raw = fs::read_to_string(run_path(run_id))
        .with_context(|| format!("failed to read run {}", run_id))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse run {}", run_id))
}

pub fn load_global_provider_cooldowns() -> Result<BTreeMap<String, ProviderCooldown>> {
    let path = provider_cooldowns_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn write_global_provider_cooldowns(
    cooldowns: &BTreeMap<String, ProviderCooldown>,
) -> Result<()> {
    write_json_atomic(&provider_cooldowns_path(), cooldowns)
}

pub fn load_task(run_id: &str, task_id: &str) -> Result<CoordinationTask> {
    let raw = fs::read_to_string(task_path(run_id, task_id))
        .with_context(|| format!("failed to read task {} for run {}", task_id, run_id))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse task {} for run {}", task_id, run_id))
}

pub fn load_claim(run_id: &str, task_id: &str) -> Result<TaskClaim> {
    let raw = fs::read_to_string(claim_path(run_id, task_id))
        .with_context(|| format!("failed to read claim {} for run {}", task_id, run_id))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse claim {} for run {}", task_id, run_id))
}

pub fn load_tasks(run_id: &str) -> Result<Vec<CoordinationTask>> {
    let mut tasks = Vec::new();
    let entries = match fs::read_dir(tasks_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(tasks),
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // Ignore atomic-write scratch files like `.task.json.tmp-12345`.
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read task {}", path.display()))?;
        let task: CoordinationTask = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse task {}", path.display()))?;
        tasks.push(task);
    }

    tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(tasks)
}

pub fn seed_initial_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    match run.workflow_mode {
        AutonomousWorkflowMode::DesignConsensus => seed_design_consensus_tasks(run),
        AutonomousWorkflowMode::DesignImplementation => seed_design_implementation_tasks(run),
        AutonomousWorkflowMode::LightweightImplementation => {
            seed_lightweight_implementation_tasks(run)
        }
        AutonomousWorkflowMode::GithubBranchFix => seed_github_branch_fix_tasks(run),
        AutonomousWorkflowMode::SingleProvider => seed_single_provider_tasks(run),
        AutonomousWorkflowMode::RalphLoop => seed_ralph_loop_tasks(run),
        // Council mode does not use the task DAG — it has its own ledger-based driver.
        AutonomousWorkflowMode::FreeformCouncil => Vec::new(),
        AutonomousWorkflowMode::CodeReview => seed_code_review_tasks(run),
        AutonomousWorkflowMode::PerfHillClimber => seed_perf_hill_climber_tasks(run),
    }
}

pub fn seed_single_provider_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    let survey_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let review_id = Uuid::new_v4().to_string();
    let assigned_provider = Some(run.provider.clone());
    let mut tasks = vec![
        make_task(
            run,
            &survey_id,
            None,
            CoordinationTaskKind::SurveyWorkspace,
            None,
            "Survey workspace".to_string(),
            format!(
                "Survey the workspace for context relevant to this autonomous goal: {}",
                goal
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string()],
            Vec::new(),
            Vec::new(),
            CoordinationTaskStatus::Queued,
        ),
        make_task(
            run,
            &plan_id,
            Some(survey_id.clone()),
            CoordinationTaskKind::PlanFlatTasks,
            assigned_provider.clone(),
            "Plan flat tasks".to_string(),
            format!(
                "Use the survey results to break this goal into flat, non-overlapping tasks: {}.\n\n\
                Return JSON only, with no markdown fences or commentary. The JSON must be an array of objects.\n\n\
                JSON template (fill in your values):\n\
                [\n  {{\"title\": \"Task title\", \"prompt\": \"What to do\", \"kind\": \"write\", \"read_scope\": [\"**\"], \"write_scope\": [\"**\"], \"allowed_tools\": [\"filesystem\", \"edit\"], \"acceptance_checks\": [\"check 1\", \"check 2\"]}}\n]\n\n\
                Rules:\n\
                - Do not include a `provider` field; provider assignment is handled by the coordination engine.\n\
                - `kind` must be `read` or `write`.\n\
                - Use `read` for analysis, inspection, or evidence-gathering tasks.\n\
                - Use `write` only for tasks that should make code or file changes.\n\
                - Keep tasks independent and non-overlapping.\n\
                - Make write tasks small: one concrete capability per write task.\n\
                - Set `read_scope` to `[\"**\"]` for every generated task.\n\
                - Set `write_scope` to `[\"**\"]` for every `write` task; `read` tasks should keep `write_scope` empty.\n\
                - Patch-level git publish is the consolidation boundary, so do not try to narrow file ownership in task scopes.\n\
                - `allowed_tools`, `read_scope`, `write_scope`, and `acceptance_checks` must be JSON arrays of strings.\n\
                - Every `write` task must include at least 2 concrete `acceptance_checks`.\n\
                - Prefer 2-6 concrete tasks over broad generic tasks.",
                goal
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string()],
            Vec::new(),
            vec![survey_id.clone()],
            CoordinationTaskStatus::Blocked,
        ),
        make_task(
            run,
            &review_id,
            Some(plan_id.clone()),
            CoordinationTaskKind::ReviewArtifacts,
            assigned_provider.clone(),
            "Review artifacts and decide apply steps".to_string(),
            "Review produced artifacts and decide which non-overlapping tasks should be applied next."
                .to_string(),
            vec!["filesystem".to_string()],
            vec!["artifacts/**".to_string()],
            if run.allow_write_tasks {
                vec![".".to_string()]
            } else {
                Vec::new()
            },
            vec![plan_id],
            CoordinationTaskStatus::Blocked,
        ),
    ];

    if run.allow_write_tasks {
        let apply_id = Uuid::new_v4().to_string();
        tasks.push(make_task(
            run,
            &apply_id,
            Some(review_id.clone()),
            CoordinationTaskKind::ApplyApprovedEdits,
            assigned_provider,
            "Apply non-overlapping edit tasks".to_string(),
            "Apply the approved, non-overlapping edit tasks using the dependency artifacts as your plan. You may edit files inside the declared write scope only, and you should make the smallest complete set of changes needed.".to_string(),
            vec!["filesystem".to_string(), "edit".to_string()],
            vec!["**".to_string()],
            vec![".".to_string()],
            vec![review_id],
            CoordinationTaskStatus::Blocked,
        ));
    }

    tasks
}

pub fn seed_ralph_loop_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    // Create both template files before tasks run
    let verify_template = r#"{
  "complete": false,
  "reason": "Enter reason here - explain what remains to be done"
}"#;
    let verify_path = PathBuf::from(ralph_verify_template_path(&run.run_id));
    if let Err(e) = std::fs::write(&verify_path, verify_template) {
        warn!(
            path = %verify_path.display(),
            error = %e,
            "failed to create Ralph verification template"
        );
    }

    let execute_template = r#"{
  "outcome": "completed_no_change",
  "summary": "All criteria already satisfied",
  "changed_files": [],
  "blockers": [],
  "reason": ""
}"#;
    let execute_path = PathBuf::from(ralph_execute_template_path(&run.run_id));
    if let Err(e) = std::fs::write(&execute_path, execute_template) {
        warn!(
            path = %execute_path.display(),
            error = %e,
            "failed to create Ralph execute template"
        );
    }

    let survey_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let execute_id = Uuid::new_v4().to_string();
    let verify_id = Uuid::new_v4().to_string();
    let assigned_provider = Some(run.provider.clone());

    let mut plan_task = make_task(
        run,
        &plan_id,
        Some(survey_id.clone()),
        CoordinationTaskKind::RalphPlan,
        assigned_provider.clone(),
        "Plan Ralph iteration 1".to_string(),
        build_ralph_plan_prompt(run, 1),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec![survey_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    plan_task.iteration = Some(1);

    let mut execute_task = make_task(
        run,
        &execute_id,
        Some(plan_id.clone()),
        CoordinationTaskKind::RalphExecute,
        assigned_provider.clone(),
        "Ralph iteration 1".to_string(),
        build_ralph_execute_prompt(run, 1),
        ralph_execute_tools(run.allow_write_tasks),
        vec!["**".to_string(), "artifacts/**".to_string()],
        ralph_execute_write_scope(&run.run_id, run.allow_write_tasks),
        vec![plan_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    execute_task.iteration = Some(1);

    let mut verify_task = make_task(
        run,
        &verify_id,
        Some(execute_id.clone()),
        CoordinationTaskKind::RalphVerify,
        assigned_provider,
        "Verify Ralph iteration 1".to_string(),
        build_ralph_verify_prompt(run, 1),
        ralph_verify_tools(),
        ralph_verify_read_scope(&run.run_id),
        ralph_verify_write_scope(&run.run_id),
        vec![plan_id.clone(), execute_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    verify_task.iteration = Some(1);

    vec![
        make_task(
            run,
            &survey_id,
            None,
            CoordinationTaskKind::SurveyWorkspace,
            None,
            "Survey workspace".to_string(),
            format!(
                "Survey the workspace for context relevant to this autonomous goal: {}",
                goal
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string()],
            Vec::new(),
            Vec::new(),
            CoordinationTaskStatus::Queued,
        ),
        plan_task,
        execute_task,
        verify_task,
    ]
}

pub fn seed_design_consensus_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    let workspace_root = prompt_workspace_root_for_run(run);
    let survey_id = Uuid::new_v4().to_string();
    let converge_id = Uuid::new_v4().to_string();
    let mut tasks = vec![make_task(
        run,
        &survey_id,
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace".to_string(),
        format!(
            "Survey the workspace for context relevant to this autonomous goal: {}",
            goal
        ),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Queued,
    )];
    let mut draft_ids = Vec::new();

    for provider in resolve_design_draft_providers(run) {
        let provider_label = provider_display_name(&provider);
        let task_id = Uuid::new_v4().to_string();
        draft_ids.push(task_id.clone());
        let draft_doc_path = provider_draft_design_doc_path(run, &provider);
        let base_prompt = format!(
            "Inspect the workspace at {} and propose the best single design for this goal: {}.\n\nThere are multiple design docs already. Your job is to review the existing design material, identify the strongest path forward, and write one provider-specific proposal artifact in markdown.\n\nRequirements:\n- Focus on the repo state that actually exists today.\n- Call out which existing design ideas should be kept, changed, or discarded.\n- Optimize for an implementable plan, not a brainstorming list.\n- Include sections: Current State, Recommended Design, Rejected Alternatives, Implementation Slices, Risks.\n- If you discuss provider weaknesses, failed runs, or evaluation gaps, anchor that analysis to the current autonomous providers configured for this repo by reading `workflow-registry/providers.json`.\n- Do not design around failures of weaker, out-of-band, or otherwise unrelated models just because they are easier to fail.\n- Assume the canonical converged design doc will be written to `{}`.",
            workspace_root, goal, run.design_doc_path
        );
        let mut task = make_task(
            run,
            &task_id,
            Some(survey_id.clone()),
            CoordinationTaskKind::DraftDesignDoc,
            Some(provider.clone()),
            format!("Draft provider design proposal with {}", provider_label),
            apply_prompt_family(&provider, &base_prompt),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string()],
            Vec::new(),
            vec![survey_id.clone()],
            CoordinationTaskStatus::Blocked,
        );
        task.task_category = Some(TaskCategory::Design);
        task.provider_hint = Some(provider.clone());

        if seed_existing_design_draft_artifact(run, &mut task, &draft_doc_path) {
            tasks.push(task);
            continue;
        }

        tasks.push(task);
    }

    // Converge task: resolve provider via category routing
    let converge_provider =
        resolve_provider_for_category(run, &TaskCategory::Design, Some("claude"))
            .unwrap_or_else(|| "claude".to_string());
    let converge_base_prompt = format!(
        "Synthesize the provider design proposal artifacts into a single canonical design doc for this goal: {}.\n\nReturn markdown only.\n\nRequirements:\n- Produce one final design doc suitable to implement immediately.\n- Resolve disagreements explicitly instead of listing incompatible options.\n- The design doc should target `{}` inside the workspace.\n- Include sections: Goal, Chosen Design, Why This Design, Detailed Plan, Implementation Order, Risks, Validation.\n- Any discussion of provider weaknesses, failed runs, or evaluation evidence must target the current autonomous providers configured for this repo, not weaker or unrelated external models.\n- Treat this output as the single source of truth for follow-on implementation planning.",
        goal, run.design_doc_path
    );
    let mut converge_task = make_task(
        run,
        &converge_id,
        Some(survey_id.clone()),
        CoordinationTaskKind::ConvergeDesignDoc,
        Some(converge_provider.clone()),
        "Converge on one canonical design doc".to_string(),
        apply_prompt_family(&converge_provider, &converge_base_prompt),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["artifacts/**".to_string(), "**".to_string()],
        Vec::new(),
        draft_ids.clone(),
        CoordinationTaskStatus::Blocked,
    );
    converge_task.task_category = Some(TaskCategory::Design);
    tasks.push(converge_task);

    // DesignConsensus stops here — materialize the doc then done. No planning or implementation.
    if run.allow_write_tasks {
        let materialize_id = Uuid::new_v4().to_string();
        let materialize_provider = "codex".to_string();
        let materialize_base_prompt = format!(
            "Use the converged design doc artifact and write the canonical design doc to `{}`.\n\nKeep the write limited to that file. If the file already exists, replace its contents so there is one canonical document to implement from.",
            run.design_doc_path
        );
        let mut mat_task = make_task(
            run,
            &materialize_id,
            Some(converge_id.clone()),
            CoordinationTaskKind::MaterializeDesignDoc,
            Some(materialize_provider.clone()),
            "Write canonical design doc to the workspace".to_string(),
            apply_prompt_family(&materialize_provider, &materialize_base_prompt),
            vec!["filesystem".to_string(), "edit".to_string()],
            vec!["**".to_string(), "artifacts/**".to_string()],
            vec![run.design_doc_path.clone()],
            vec![converge_id.clone()],
            CoordinationTaskStatus::Blocked,
        );
        mat_task.task_category = Some(TaskCategory::DocWrite);
        tasks.push(mat_task);
    }

    tasks
}

pub fn seed_existing_design_draft_artifact(
    run: &CoordinationRun,
    task: &mut CoordinationTask,
    draft_doc_path: &str,
) -> bool {
    let absolute_path = workspace_path_for_run(run, draft_doc_path);
    if !absolute_path.is_file() {
        return false;
    }

    let Ok(body) = fs::read_to_string(&absolute_path) else {
        return false;
    };

    let summary = format!(
        "Reused existing provider draft design doc at `{}`.",
        draft_doc_path
    );
    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: task.assigned_provider.clone(),
        task_category: task.task_category,
        fallback_from: task.provider_hint.clone(),
        summary: summary.clone(),
        body: body.clone(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: summary.clone(),
            details: body,
            design_requirements_satisfied: vec![
                "Reused existing provider draft design doc already present in the workspace."
                    .to_string(),
            ],
            ..Default::default()
        },
    };

    if fs::create_dir_all(artifact_dir(&run.run_id, &task.task_id)).is_err() {
        return false;
    }
    if write_json_atomic(&artifact_result_path(&run.run_id, &task.task_id), &artifact).is_err() {
        return false;
    }
    if fs::write(
        artifact_notes_path(&run.run_id, &task.task_id),
        &artifact.body,
    )
    .is_err()
    {
        return false;
    }

    task.status = CoordinationTaskStatus::Completed;
    task.artifact_summary = Some(format!(
        "{} [{}]",
        artifact.summary,
        artifact.report.outcome.as_str()
    ));
    task.updated_at = Utc::now();
    true
}

/// Seeds the task DAG for DesignImplementation: takes an existing design doc and implements it.
/// Skips design convergence — goes straight to survey → plan → execute → review → apply.
pub fn seed_design_implementation_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    let survey_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let review_id = Uuid::new_v4().to_string();

    let mut tasks = vec![make_task(
        run,
        &survey_id,
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace".to_string(),
        format!(
            "Survey the workspace and read the canonical design doc at `{}` for context relevant to this implementation goal: {}",
            run.design_doc_path, goal
        ),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Queued,
    )];

    // Plan task: break design doc into implementation tasks
    let plan_provider = resolve_provider_for_category(run, &TaskCategory::Plan, Some("codex"))
        .unwrap_or_else(|| "codex".to_string());
    let plan_base_prompt = format!(
        "Use the canonical design doc at `{}` and workspace evidence to break this goal into non-overlapping implementation tasks: {}.\n\nReturn JSON only, with no markdown fences or commentary. The JSON must be an array of objects. Each object must contain: `title`, `prompt`, `kind`, `read_scope`, `write_scope`, `allowed_tools`, and `acceptance_checks`.\n\nRules:\n- Do not include a `provider` field; provider assignment is handled by the coordination engine.\n- The coordination engine will distribute runnable tasks across available providers.\n- `kind` must be `read` or `write`.\n- `write` tasks must be implementation tasks that follow the canonical design doc at `{}`.\n- Keep write tasks small: one concrete capability per task.\n- Set `read_scope` to `[\"**\"]` for every generated task.\n- Set `write_scope` to `[\"**\"]` for every `write` task; `read` tasks should keep `write_scope` empty.\n- Patch-level git publish is the consolidation boundary, so do not try to narrow file ownership in task scopes.\n- `allowed_tools`, `read_scope`, `write_scope`, and `acceptance_checks` must be JSON arrays of strings.\n- Every `write` task must include at least 2 concrete `acceptance_checks`.\n- Prefer 3-8 concrete implementation slices over broad generic tasks.",
        run.design_doc_path, goal, run.design_doc_path
    );
    let mut plan_task = make_task(
        run,
        &plan_id,
        Some(survey_id.clone()),
        CoordinationTaskKind::PlanFlatTasks,
        Some(plan_provider.clone()),
        "Plan non-overlapping implementation tasks from design doc".to_string(),
        apply_prompt_family(&plan_provider, &plan_base_prompt),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string(), "artifacts/**".to_string()],
        Vec::new(),
        vec![survey_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    plan_task.task_category = Some(TaskCategory::Plan);
    tasks.push(plan_task);

    // Review task
    let review_provider = resolve_provider_for_category(run, &TaskCategory::Review, Some("claude"))
        .unwrap_or_else(|| "claude".to_string());
    let review_base_prompt = format!(
        "Review the implementation artifacts against the canonical design doc at `{}` and decide whether the run is compliant. If there are gaps, emit smaller follow-up fix tasks instead of broad recommendations.\n\n\
        Return JSON only, with no markdown fences or commentary.\n\n\
        JSON template (fill in your values):\n\
        {{\"outcome\": \"completed_with_changes\", \"summary\": \"Brief description\", \"details\": \"What was done\", \"changed_files\": [\"file1.rs\"], \"tests_run\": [], \"warnings\": [], \"blockers\": [], \"remaining_gaps\": [], \"design_requirements_satisfied\": [\"req1\"], \"fix_tasks\": []}}",
        run.design_doc_path
    );
    let mut review_task = make_task(
        run,
        &review_id,
        Some(plan_id.clone()),
        CoordinationTaskKind::ReviewArtifacts,
        Some(review_provider.clone()),
        "Review artifacts against design doc".to_string(),
        apply_prompt_family(&review_provider, &review_base_prompt),
        vec!["filesystem".to_string()],
        vec!["artifacts/**".to_string(), run.design_doc_path.clone()],
        if run.allow_write_tasks {
            vec![".".to_string()]
        } else {
            Vec::new()
        },
        vec![plan_id],
        CoordinationTaskStatus::Blocked,
    );
    review_task.task_category = Some(TaskCategory::Review);
    tasks.push(review_task);

    if run.allow_write_tasks {
        let apply_id = Uuid::new_v4().to_string();
        let apply_provider =
            resolve_provider_for_category(run, &TaskCategory::DeepWrite, Some("codex"))
                .unwrap_or_else(|| "codex".to_string());
        let apply_base_prompt = format!(
            "Apply the approved, non-overlapping edit tasks using the canonical design doc at `{}` and the dependency artifacts as your plan. You may edit files inside the declared write scope only, and you should make the smallest complete set of changes needed.",
            run.design_doc_path
        );
        let mut apply_task = make_task(
            run,
            &apply_id,
            Some(review_id.clone()),
            CoordinationTaskKind::ApplyApprovedEdits,
            Some(apply_provider.clone()),
            "Apply non-overlapping edit tasks".to_string(),
            apply_prompt_family(&apply_provider, &apply_base_prompt),
            vec!["filesystem".to_string(), "edit".to_string()],
            vec!["**".to_string()],
            vec![".".to_string()],
            vec![review_id],
            CoordinationTaskStatus::Blocked,
        );
        apply_task.task_category = Some(TaskCategory::DeepWrite);
        tasks.push(apply_task);
    }

    tasks
}

pub fn seed_lightweight_implementation_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let goal = provider_prompt_text_for_run(run, &run.goal);
    let config = lightweight_config_from_run(run);
    let design_doc_exists = workspace_path_for_run(run, &run.design_doc_path).is_file();
    let task_doc_exists = workspace_path_for_run(run, &config.task_doc_path).is_file();
    let survey_id = Uuid::new_v4().to_string();
    let rust_scaffold_conversion_id = Uuid::new_v4().to_string();
    let implementation_plan_id = Uuid::new_v4().to_string();
    let validation_plan_id = Uuid::new_v4().to_string();
    let review_id = Uuid::new_v4().to_string();
    let repo_configuration_guidance = match config.repo_configuration {
        LightweightRepoConfiguration::Rust => {
            let scaffold_root = super::workspace_ready::rust_scaffold_root();
            format!(
                "\n\nRust scaffold rule:\n- Use `{}` as the canonical Rust project baseline (configurable via AUTO_RUNS_RUST_SCAFFOLD_ROOT).\n- If the workspace is fresh or only contains `docs/`, the runtime seeds the full scaffold tree except `.git` and `target`.\n- Before feature work, run a dedicated LLM conversion step that updates scaffold names, workspace members, package identifiers, app names, helper scripts, docs, and copied references so they match the current repository instead of the scaffold source project.\n- If the workspace already contains files beyond `docs/`, do not assume the runtime will overwrite them. The dedicated conversion step should inspect the existing repo state, align it toward the scaffold baseline where appropriate, and then rename copied scaffold references for the current repository.",
                scaffold_root
            )
        }
        _ => String::new(),
    };

    let mut tasks = vec![make_task(
        run,
        &survey_id,
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Survey workspace".to_string(),
        format!(
            "Survey the workspace for context relevant to this lightweight implementation goal: {}. The canonical design doc target is `{}` and the active task list target is `{}`. Treat that task list as read-only planning input for the current pass: implementation workers must not edit it, and review may emit a new next-pass task list only after a full reassessment. If either document does not exist yet, note that the first implementation planning pass must bootstrap it from this goal and the workspace state.",
            goal, run.design_doc_path, config.task_doc_path
        ),
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Queued,
    )];
    let implementation_plan_dependencies = if config.repo_configuration
        == LightweightRepoConfiguration::Rust
    {
        let conversion_provider = config
            .worker_providers
            .first()
            .cloned()
            .unwrap_or_else(|| "codex".to_string());
        let scaffold_root = super::workspace_ready::rust_scaffold_root();
        let conversion_prompt = format!(
            "Inspect the current Rust workspace, the canonical scaffold at `{}` (configurable via AUTO_RUNS_RUST_SCAFFOLD_ROOT), the design doc target `{}`, and the active task list `{}`. Convert the scaffolded repository so it belongs to this repo instead of the scaffold source project.\n\nRequirements:\n- Update scaffold names, workspace members, package identifiers, crate names, binary names, docs, helper scripts, and copied references so they consistently match the current repository goal: {}.\n- If the workspace was freshly scaffolded, treat this as the first normalization pass before feature work.\n- If the workspace already had files beyond `docs/`, align the existing repo toward the scaffold baseline only where it helps the current repo, and preserve repo-specific work that should remain.\n- Do not edit `{}`; review owns task-list updates and next-pass task-list creation.\n- Keep the result ready for follow-on feature tasks, but do not implement unrelated product features yet.\n\nReturn the normal write-task report format only.",
            scaffold_root,
            run.design_doc_path,
            config.task_doc_path,
            goal,
            config.task_doc_path
        );
        let mut conversion_task = make_task(
            run,
            &rust_scaffold_conversion_id,
            Some(survey_id.clone()),
            CoordinationTaskKind::GeneratedWriteTask,
            Some(conversion_provider),
            "Convert scaffolded Rust workspace to current repo".to_string(),
            conversion_prompt,
            vec![
                "filesystem".to_string(),
                "search".to_string(),
                "edit".to_string(),
                "sequentialthinking".to_string(),
            ],
            vec![
                "**".to_string(),
                run.design_doc_path.clone(),
                config.task_doc_path.clone(),
            ],
            vec![
                ".cargo".to_string(),
                ".githooks".to_string(),
                ".gitignore".to_string(),
                ".dockerignore".to_string(),
                "Cargo.lock".to_string(),
                "Cargo.toml".to_string(),
                "Dockerfile".to_string(),
                "LICENSE".to_string(),
                "README.md".to_string(),
                "apps".to_string(),
                "config".to_string(),
                "crates".to_string(),
                "docs".to_string(),
                "scripts".to_string(),
                "rust-toolchain.toml".to_string(),
                "rustfmt.toml".to_string(),
            ],
            vec![survey_id.clone()],
            CoordinationTaskStatus::Blocked,
        );
        conversion_task.task_category = Some(TaskCategory::DeepWrite);
        tasks.push(conversion_task);
        vec![rust_scaffold_conversion_id.clone()]
    } else {
        vec![survey_id.clone()]
    };

    let implementation_plan_prompt = format!(
        "{}\n\nUse this implementation idea as the starting point: {}.\n\nUse the workspace survey, the canonical design doc path `{}`, and the current task list path `{}` to split the remaining implementation work into small, concrete fragments for worker models.{}\n\nIf `{}` or `{}` does not exist yet, bootstrap this workflow first:\n- create the canonical design doc at `{}` from the goal and current workspace state\n- create the initial task list at `{}`\n- then emit the current-pass implementation fragments against those documents\n\nReturn JSON only, with no markdown fences or commentary. The JSON must be an array of task objects.\n\nRules:\n- Emit only implementation tasks for the current pass.\n- Keep tasks small and non-overlapping.\n- Use `kind: \"write\"` for implementation fragments that modify the repo.\n- Use `kind: \"read\"` only when a worker must inspect or gather evidence before a later write.\n- For every write task, make `title` the exact git commit message the runtime should reuse verbatim after the task writes files.\n- Start `title` with an imperative subject line. If the task intentionally bundles multiple features, additional non-empty lines are allowed.\n- Every generated task must set a realistic integer `estimated_minutes` between 1 and 180.\n- If a fragment depends on another generated task, set `depends_on_titles` to the exact upstream task titles instead of relying on prose-only ordering notes.\n- Set `read_scope` to `[\"**\"]` for every generated task.\n- Set `write_scope` to `[\"**\"]` for every `write` task; `read` tasks should keep `write_scope` empty.\n- Patch-level git publish is the consolidation boundary, so do not try to narrow file ownership in task scopes.\n- Generated implementation workers must not edit the current task list.\n- Review owns task-list progression, reassessment, and next-pass task-list creation.\n- The current pass ends when review determines the workspace satisfies the current task list and design intent, or after review creates the next task list from a full reassessment.\n- Use the sequential thinking MCP tool when helpful to sequence dependencies and break the work into concrete fragments.\n- Do not enforce any extra task formatting beyond what the runtime contract requires.\n- Prefer 3-10 concrete fragments over broad umbrella tasks.",
        lightweight_prompt_prefix(&config.task_doc_path),
        goal,
        run.design_doc_path,
        config.task_doc_path,
        repo_configuration_guidance,
        run.design_doc_path,
        config.task_doc_path,
        run.design_doc_path,
        config.task_doc_path
    );
    let mut implementation_plan = make_task(
        run,
        &implementation_plan_id,
        Some(survey_id.clone()),
        CoordinationTaskKind::PlanFlatTasks,
        Some(config.task_splitter_provider.clone()),
        "Split lightweight implementation tasks".to_string(),
        implementation_plan_prompt,
        if run.allow_write_tasks {
            vec![
                "filesystem".to_string(),
                "search".to_string(),
                "edit".to_string(),
                "sequentialthinking".to_string(),
            ]
        } else {
            vec![
                "filesystem".to_string(),
                "search".to_string(),
                "sequentialthinking".to_string(),
            ]
        },
        vec!["**".to_string(), "artifacts/**".to_string()],
        if run.allow_write_tasks && (!design_doc_exists || !task_doc_exists) {
            vec![run.design_doc_path.clone(), config.task_doc_path.clone()]
        } else {
            Vec::new()
        },
        implementation_plan_dependencies,
        CoordinationTaskStatus::Blocked,
    );
    implementation_plan.task_category = Some(TaskCategory::Plan);
    tasks.push(implementation_plan);

    let validation_plan_prompt = format!(
        "{}\n\nAfter the implementation fragments finish, break validation into small read-only tasks that check the completed work against `{}` and `{}`.\n\nReturn JSON only, with no markdown fences or commentary. The JSON must be an array of task objects.\n\nRules:\n- Emit only validation tasks for the current pass.\n- Every validation task must use `kind: \"read\"`.\n- Every validation task must set a realistic integer `estimated_minutes` between 1 and 180.\n- Validation tasks must inspect the actual workspace state, not just prior artifacts.\n- Keep validation tasks narrow and evidence-driven.\n- The validation pass should produce enough evidence for a final review to either approve completion or, after a full reassessment, update `{}` and produce the next task list document.\n- Use the sequential thinking MCP tool when helpful to structure the reassessment evidence.\n- Do not emit write tasks from this planning step.",
        lightweight_prompt_prefix(&config.task_doc_path),
        run.design_doc_path,
        config.task_doc_path,
        run.design_doc_path
    );
    let mut validation_plan = make_task(
        run,
        &validation_plan_id,
        Some(implementation_plan_id.clone()),
        CoordinationTaskKind::PlanFlatTasks,
        Some(config.validation_provider.clone()),
        "Split lightweight validation tasks".to_string(),
        validation_plan_prompt,
        vec![
            "filesystem".to_string(),
            "search".to_string(),
            "sequentialthinking".to_string(),
        ],
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
    tasks.push(validation_plan);

    let mut review_task = make_task(
        run,
        &review_id,
        Some(validation_plan_id.clone()),
        CoordinationTaskKind::ReviewArtifacts,
        Some(config.validation_provider),
        "Validate lightweight implementation pass".to_string(),
        lightweight_review_prompt(run),
        if run.allow_write_tasks {
            vec!["filesystem".to_string(), "edit".to_string()]
        } else {
            vec!["filesystem".to_string()]
        },
        vec![
            "artifacts/**".to_string(),
            "**".to_string(),
            run.design_doc_path.clone(),
        ],
        lightweight_review_write_scope(run),
        vec![validation_plan_id],
        CoordinationTaskStatus::Blocked,
    );
    review_task.task_category = Some(TaskCategory::Review);
    tasks.push(review_task);

    tasks
}
