pub fn seed_code_review_tasks(run: &CoordinationRun) -> Vec<CoordinationTask> {
    let survey_id = Uuid::new_v4().to_string();
    let plan_id = Uuid::new_v4().to_string();
    let materialize_id = Uuid::new_v4().to_string();
    let output_path = run.design_doc_path.clone();
    let scope_description =
        code_review::describe_scope(run.review_scope.as_ref().unwrap_or(&ReviewScope::Worktree));
    let mut materialize = make_task(
        run,
        &materialize_id,
        Some(plan_id.clone()),
        CoordinationTaskKind::MaterializeDesignDoc,
        None,
        "Write code review document".to_string(),
        format!(
            "Write the final code review document to `{}` using the completed review chunk artifacts.",
            output_path
        ),
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["artifacts/**".to_string()],
        vec![output_path.clone()],
        vec![plan_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    materialize.acceptance_checks = vec![
        format!("The final review document is written to {}.", output_path),
        "Findings are severity-ordered and cite file:line references.".to_string(),
    ];

    vec![
        make_task(
            run,
            &survey_id,
            None,
            CoordinationTaskKind::SurveyWorkspace,
            None,
            "Survey workspace".to_string(),
            format!(
                "Survey the workspace for code review context relevant to this review scope: {}",
                scope_description
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
            CoordinationTaskKind::PlanCodeReview,
            None,
            "Plan code review chunks".to_string(),
            format!(
                "Plan a code review for {} and emit read-only review chunks using `{}`.",
                scope_description,
                code_review::REVIEW_PROFILE_PATH
            ),
            vec!["filesystem".to_string(), "search".to_string()],
            vec!["**".to_string(), "artifacts/**".to_string()],
            Vec::new(),
            vec![survey_id],
            CoordinationTaskStatus::Blocked,
        ),
        materialize,
    ]
}

pub fn persist_seed_code_review_metadata(
    run: &CoordinationRun,
    tasks: &[CoordinationTask],
) -> Result<()> {
    let scope_description =
        code_review::describe_scope(run.review_scope.as_ref().unwrap_or(&ReviewScope::Worktree));
    let fix_mode = run.review_fix_mode.clone().unwrap_or_default();
    for task in tasks {
        if task.kind != CoordinationTaskKind::MaterializeDesignDoc {
            continue;
        }
        let output_path = task
            .write_scope
            .first()
            .cloned()
            .unwrap_or_else(|| run.design_doc_path.clone());
        write_review_task_metadata(
            &run.run_id,
            &task.task_id,
            &code_review::ReviewTaskMetadata {
                scope_description: scope_description.clone(),
                dimensions: run.review_dimensions.clone().unwrap_or_default(),
                chunk: Vec::new(),
                automated_checks: Vec::new(),
                profile_path: code_review::REVIEW_PROFILE_PATH.to_string(),
                profile_warnings: Vec::new(),
                fix_mode: Some(fix_mode.clone()),
                output_path: Some(output_path.clone()),
            },
        )?;
    }
    Ok(())
}

pub fn make_task(
    run: &CoordinationRun,
    task_id: &str,
    parent_task_id: Option<String>,
    kind: CoordinationTaskKind,
    assigned_provider: Option<String>,
    title: String,
    prompt: String,
    allowed_tools: Vec<String>,
    read_scope: Vec<String>,
    write_scope: Vec<String>,
    dependencies: Vec<String>,
    status: CoordinationTaskStatus,
) -> CoordinationTask {
    let now = Utc::now();
    let acceptance_checks = seeded_acceptance_checks(&kind, &write_scope);
    CoordinationTask {
        provider_hint: None,
        required_role: None,
        task_category: None,
        task_id: task_id.to_string(),
        run_id: run.run_id.clone(),
        parent_task_id,
        kind,
        assigned_provider,
        iteration: None,
        title,
        prompt,
        cwd: perf_workspace_path_for_run(run),
        allowed_tools,
        read_scope,
        write_scope,
        acceptance_checks,
        review_metadata: None,
        dependencies,
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 0,
        max_attempts: default_task_max_attempts(),
        publish_attempt_count: 0,
        estimated_minutes: Some(default_task_estimated_minutes_for_kind(&kind)),
        status,
        claimed_by: None,
        artifact_summary: None,
        failure_kind: None,
        recovery_action: None,
        created_at: now,
        updated_at: now,
    }
}

/// Load the provider capability registry from `providers.json`.
/// Falls back to the embedded checked-in registry if the file is missing or unparseable.
pub fn load_capability_registry() -> ProviderCapabilityRegistry {
    const EMBEDDED_PROVIDER_CAPABILITY_REGISTRY_JSON: &str =
        include_str!("../../../workflow-registry/providers.json");

    match std::fs::read_to_string(PROVIDER_CAPABILITY_REGISTRY_PATH) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_else(|_| {
            serde_json::from_str(EMBEDDED_PROVIDER_CAPABILITY_REGISTRY_JSON)
                .expect("embedded provider capability registry should deserialize")
        }),
        Err(_) => serde_json::from_str(EMBEDDED_PROVIDER_CAPABILITY_REGISTRY_JSON)
            .expect("embedded provider capability registry should deserialize"),
    }
}

pub fn find_provider_profile<'a>(
    registry: &'a ProviderCapabilityRegistry,
    provider_id: &str,
) -> Option<&'a ProviderCapabilityProfile> {
    registry
        .providers
        .iter()
        .find(|profile| profile.provider_id == provider_id)
}

pub fn provider_supports_category(
    registry: &ProviderCapabilityRegistry,
    provider_id: &str,
    category: &TaskCategory,
) -> bool {
    find_provider_profile(registry, provider_id)
        .map(|profile| profile.supported_categories.contains(category))
        .unwrap_or(false)
}

pub fn push_unique_provider(providers: &mut Vec<String>, provider_id: &str) {
    if !providers.iter().any(|existing| existing == provider_id) {
        providers.push(provider_id.to_string());
    }
}

pub fn ordered_available_providers_for_category(
    run: &CoordinationRun,
    category: &TaskCategory,
    now: DateTime<Utc>,
) -> Vec<String> {
    let registry = load_capability_registry();
    let mut ordered = Vec::new();

    for profile in &registry.providers {
        if profile.preferred_categories.contains(category)
            && profile.supported_categories.contains(category)
            && is_provider_on_cooldown(run, &profile.provider_id, now).is_none()
        {
            push_unique_provider(&mut ordered, &profile.provider_id);
        }
    }

    for profile in &registry.providers {
        if profile.supported_categories.contains(category)
            && is_provider_on_cooldown(run, &profile.provider_id, now).is_none()
        {
            push_unique_provider(&mut ordered, &profile.provider_id);
        }
    }

    ordered
}

/// Resolve providers for design-consensus draft tasks using capability routing.
/// Returns provider IDs whose profile supports the `Design` category, preferring
/// those that list it as a preferred category. Respects cooldowns.
pub fn resolve_design_draft_providers(run: &CoordinationRun) -> Vec<String> {
    let now = Utc::now();
    let category = TaskCategory::Design;
    let registry = load_capability_registry();
    let mut preferred = ordered_available_providers_for_category(run, &category, now);

    // Ensure at least one provider is available
    if preferred.is_empty() {
        for profile in &registry.providers {
            if is_provider_on_cooldown(run, &profile.provider_id, now).is_none() {
                push_unique_provider(&mut preferred, &profile.provider_id);
            }
        }
    }

    preferred
}

/// Resolve the best provider for a given task category from the registry.
pub fn resolve_provider_for_category(
    run: &CoordinationRun,
    category: &TaskCategory,
    hint: Option<&str>,
) -> Option<String> {
    let now = Utc::now();
    let registry = load_capability_registry();

    if let Some(hint) = hint {
        if provider_supports_category(&registry, hint, category) {
            if is_provider_on_cooldown(run, hint, now).is_none() {
                return Some(hint.to_string());
            }
            if let Some(fallback) = select_category_fallback_provider(run, hint, category, now) {
                return Some(fallback);
            }
        }
    }

    ordered_available_providers_for_category(run, category, now)
        .into_iter()
        .next()
}

/// Look up the prompt family for a provider from the registry.
pub fn provider_prompt_family(provider: &str) -> PromptFamily {
    load_capability_registry()
        .providers
        .iter()
        .find(|p| p.provider_id == provider)
        .map(|p| p.prompt_family.clone())
        .unwrap_or(PromptFamily::Principles)
}

/// Apply prompt-family styling to a task prompt.
/// Principles-family providers get the base prompt as-is.
/// Mechanics-family providers get explicit structural guidance appended.
pub fn apply_prompt_family(provider: &str, base_prompt: &str) -> String {
    match provider_prompt_family(provider) {
        PromptFamily::Principles => base_prompt.to_string(),
        PromptFamily::Mechanics => {
            format!(
                "{}\n\nFollow these steps precisely:\n1. Read and understand the requirements above.\n2. Inspect the relevant files.\n3. Produce the requested output exactly as specified.",
                base_prompt
            )
        }
    }
}

/// Provider list for expansion.rs round-robin provider selection.
/// Resolves from the capability registry (Design-capable providers) instead of
/// a hardcoded array. Falls back to a static list only if the registry is empty.
pub fn design_consensus_provider_names() -> Vec<String> {
    let registry = load_capability_registry();
    let category = TaskCategory::Design;

    let mut result: Vec<String> = registry
        .providers
        .iter()
        .filter(|p| p.supported_categories.contains(&category))
        .map(|p| p.provider_id.clone())
        .collect();

    if result.is_empty() {
        result = vec![
            "claude".to_string(),
            "codex".to_string(),
            "minimax".to_string(),
            "gemini".to_string(),
        ];
    }

    result
}

/// Provider list for generated-task round-robin assignment across all
/// configured providers. Falls back to the built-in provider enum order if the
/// registry is unavailable or empty.
pub fn all_provider_names() -> Vec<String> {
    let registry = load_capability_registry();
    let mut result: Vec<String> = registry
        .providers
        .iter()
        .map(|p| p.provider_id.clone())
        .collect();

    if result.is_empty() {
        result = ModelProvider::ALL
            .iter()
            .map(|provider| provider.as_str().to_string())
            .collect();
    }

    result
}

pub fn provider_display_name(provider: &str) -> &'static str {
    match provider {
        "claude" => "Claude Opus",
        "glm" => "GLM 5.1",
        "codex" => "GPT-5.4",
        "minimax" => "MiniMax",
        "gemini" => "gemini-3-flash-preview",
        "qwen" => "Qwen 3.6 Free",
        _ => "Provider",
    }
}

pub fn provider_draft_design_doc_path(run: &CoordinationRun, provider: &str) -> String {
    let design_doc_path = std::path::Path::new(&run.design_doc_path);
    let parent = design_doc_path.parent();
    let stem = design_doc_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("design");
    let model = provider_model_slug(provider);
    let file_name = format!("_{}_{}.md", model, sanitize_filename_component(stem));

    match parent {
        Some(parent) if !parent.as_os_str().is_empty() => {
            parent.join(file_name).to_string_lossy().into_owned()
        }
        _ => file_name,
    }
}

pub fn prompt_workspace_root_for_run(run: &CoordinationRun) -> &str {
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        "."
    } else {
        run.workspace_path.as_str()
    }
}

pub fn provider_prompt_text_for_run(run: &CoordinationRun, text: &str) -> String {
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc
        && !run.workspace_path.trim().is_empty()
    {
        text.replace(&run.workspace_path, ".")
    } else {
        text.to_string()
    }
}

pub fn workspace_path_for_run(run: &CoordinationRun, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(&run.workspace_path).join(path)
    }
}

pub fn provider_model_slug(provider: &str) -> String {
    let registry = load_capability_registry();
    let raw = find_provider_profile(&registry, provider)
        .and_then(|profile| profile.model_override.as_deref())
        .unwrap_or(provider);
    sanitize_filename_component(raw)
}

pub fn sanitize_filename_component(raw: &str) -> String {
    let mut result = String::new();
    let mut last_was_separator = false;

    for ch in raw.trim().chars() {
        let normalized = if ch.is_ascii_alphanumeric() {
            last_was_separator = false;
            ch.to_ascii_lowercase()
        } else if matches!(ch, '.' | '-' | '_') {
            last_was_separator = false;
            ch
        } else {
            if !last_was_separator && !result.is_empty() {
                result.push('-');
            }
            last_was_separator = true;
            continue;
        };
        result.push(normalized);
    }

    let trimmed = result.trim_matches(|ch| ch == '-' || ch == '_' || ch == '.');
    if trimmed.is_empty() {
        "design".to_string()
    } else {
        trimmed.to_string()
    }
}
