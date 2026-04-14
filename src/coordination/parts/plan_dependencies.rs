use super::*;

pub fn validate_generated_task_dependency_specs(specs: &[GeneratedTaskSpec]) -> Result<()> {
    let reference_map = build_generated_task_reference_index(specs)?;

    for spec in specs {
        let self_reference = normalize_generated_task_reference(&spec.title);
        for dependency in generated_task_dependency_references(spec, reference_map.keys()) {
            let Some(target_index) = reference_map.get(&dependency) else {
                anyhow::bail!(
                    "generated task '{}' depends on unknown generated task reference '{}'",
                    spec.title,
                    dependency
                );
            };
            if normalize_generated_task_reference(&specs[*target_index].title) == self_reference {
                anyhow::bail!("generated task '{}' cannot depend on itself", spec.title);
            }
        }
    }

    Ok(())
}

pub fn apply_generated_task_dependencies(
    run: &CoordinationRun,
    plan_task_id: &str,
    generated_specs: &[(String, GeneratedTaskSpec)],
    tasks: &mut [CoordinationTask],
) {
    if generated_specs.is_empty() {
        return;
    }

    let generated_ids: BTreeSet<String> = generated_specs
        .iter()
        .map(|(task_id, _)| task_id.clone())
        .collect();
    let reference_map = build_generated_task_reference_map(generated_specs);

    for (task_id, spec) in generated_specs {
        let dependency_ids = generated_task_dependency_references(spec, reference_map.keys())
            .into_iter()
            .filter_map(|reference| reference_map.get(&reference).cloned())
            .collect::<Vec<_>>();
        if dependency_ids.is_empty() {
            continue;
        }
        let Some(task) = tasks.iter_mut().find(|task| task.task_id == *task_id) else {
            continue;
        };
        for dependency_id in dependency_ids {
            if dependency_id != task.task_id && !task.dependencies.contains(&dependency_id) {
                task.dependencies.push(dependency_id);
            }
        }
    }

    if run.workflow_mode != AutonomousWorkflowMode::DesignImplementation {
        return;
    }

    let bootstrap_task_id = tasks
        .iter()
        .filter(|task| generated_ids.contains(&task.task_id))
        .find(|task| is_bootstrap_task(task))
        .map(|task| task.task_id.clone());

    let Some(bootstrap_task_id) = bootstrap_task_id else {
        return;
    };

    for task in tasks.iter_mut() {
        if !generated_ids.contains(&task.task_id) {
            continue;
        }
        if task.task_id == bootstrap_task_id {
            continue;
        }
        if task.kind != CoordinationTaskKind::GeneratedWriteTask {
            continue;
        }
        if !task
            .dependencies
            .iter()
            .any(|dependency| dependency == plan_task_id)
        {
            continue;
        }
        if task
            .dependencies
            .iter()
            .any(|dependency| dependency == &bootstrap_task_id)
        {
            continue;
        }
        task.dependencies.push(bootstrap_task_id.clone());
    }
}

fn build_generated_task_reference_index(
    specs: &[GeneratedTaskSpec],
) -> Result<BTreeMap<String, usize>> {
    let mut references = BTreeMap::new();
    for (index, spec) in specs.iter().enumerate() {
        let normalized_title = normalize_generated_task_reference(&spec.title);
        insert_generated_task_reference(&mut references, &spec.title, index)?;
        if let Some(alias) = generated_task_title_alias(&spec.title) {
            if normalize_generated_task_reference(&alias) != normalized_title {
                insert_generated_task_reference(&mut references, &alias, index)?;
            }
        }
    }
    Ok(references)
}

fn build_generated_task_reference_map(
    generated_specs: &[(String, GeneratedTaskSpec)],
) -> BTreeMap<String, String> {
    let mut references = BTreeMap::new();
    for (task_id, spec) in generated_specs {
        let normalized_title = normalize_generated_task_reference(&spec.title);
        references.insert(normalized_title.clone(), task_id.clone());
        if let Some(alias) = generated_task_title_alias(&spec.title) {
            let normalized_alias = normalize_generated_task_reference(&alias);
            if normalized_alias != normalized_title {
                references.insert(normalized_alias, task_id.clone());
            }
        }
    }
    references
}

fn insert_generated_task_reference<T: Copy>(
    references: &mut BTreeMap<String, T>,
    reference: &str,
    value: T,
) -> Result<()> {
    let normalized = normalize_generated_task_reference(reference);
    if normalized.is_empty() {
        anyhow::bail!("generated task is missing a usable title reference");
    }
    if references.contains_key(&normalized) {
        anyhow::bail!(
            "generated task reference '{}' is ambiguous; use unique titles and aliases",
            reference.trim()
        );
    }
    references.insert(normalized, value);
    Ok(())
}

fn generated_task_dependency_references<'a>(
    spec: &GeneratedTaskSpec,
    available_refs: impl IntoIterator<Item = &'a String>,
) -> Vec<String> {
    let explicit = normalize_string_list(spec.depends_on_titles.clone())
        .into_iter()
        .map(|reference| normalize_generated_task_reference(&reference))
        .filter(|reference| !reference.is_empty())
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }

    infer_generated_task_dependency_references_from_prompt(&spec.prompt, available_refs)
}

fn infer_generated_task_dependency_references_from_prompt<'a>(
    prompt: &str,
    available_refs: impl IntoIterator<Item = &'a String>,
) -> Vec<String> {
    let known_refs = available_refs.into_iter().cloned().collect::<BTreeSet<_>>();
    let mut inferred = BTreeSet::new();
    for clause in dependency_hint_clauses(prompt) {
        infer_clause_task_aliases(&clause, &known_refs, &mut inferred);
        let lowered_clause = clause.to_ascii_lowercase();
        for reference in &known_refs {
            if reference.contains(' ') && lowered_clause.contains(reference) {
                inferred.insert(reference.clone());
            }
        }
    }
    inferred.into_iter().collect()
}

fn dependency_hint_clauses(prompt: &str) -> Vec<String> {
    let lowered = prompt.to_ascii_lowercase();
    let mut clauses = Vec::new();
    for marker in ["after ", "depends on "] {
        let mut search_from = 0;
        while let Some(relative_start) = lowered[search_from..].find(marker) {
            let start = search_from + relative_start + marker.len();
            let remaining = &prompt[start..];
            let end = remaining.find(['.', ';', '\n']).unwrap_or(remaining.len());
            clauses.push(remaining[..end].to_string());
            search_from = start;
        }
    }
    clauses
}

fn infer_clause_task_aliases(
    clause: &str,
    known_refs: &BTreeSet<String>,
    inferred: &mut BTreeSet<String>,
) {
    let lowered = clause.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b't' {
            index += 1;
            continue;
        }
        let start_digits = index + 1;
        if start_digits >= bytes.len() || !bytes[start_digits].is_ascii_digit() {
            index += 1;
            continue;
        }
        let mut cursor = start_digits;
        while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
            cursor += 1;
        }
        let start_number = lowered[start_digits..cursor].parse::<u32>().ok();
        let mut range_cursor = cursor;
        while range_cursor < bytes.len() && bytes[range_cursor].is_ascii_whitespace() {
            range_cursor += 1;
        }
        if range_cursor < bytes.len() && bytes[range_cursor] == b'-' {
            range_cursor += 1;
            while range_cursor < bytes.len() && bytes[range_cursor].is_ascii_whitespace() {
                range_cursor += 1;
            }
            if range_cursor < bytes.len() && bytes[range_cursor] == b't' {
                range_cursor += 1;
            }
            let end_digits = range_cursor;
            while range_cursor < bytes.len() && bytes[range_cursor].is_ascii_digit() {
                range_cursor += 1;
            }
            if end_digits < range_cursor {
                let end_number = lowered[end_digits..range_cursor].parse::<u32>().ok();
                if let (Some(start_number), Some(end_number)) = (start_number, end_number) {
                    for value in start_number..=end_number {
                        let alias = format!("t{value}");
                        if known_refs.contains(&alias) {
                            inferred.insert(alias);
                        }
                    }
                }
                index = range_cursor;
                continue;
            }
        }

        if let Some(start_number) = start_number {
            let alias = format!("t{start_number}");
            if known_refs.contains(&alias) {
                inferred.insert(alias);
            }
        }
        index = cursor;
    }
}

fn generated_task_title_alias(title: &str) -> Option<String> {
    let first_token = title.split_whitespace().next()?;
    let lowered = first_token.to_ascii_lowercase();
    (lowered.starts_with('t')
        && lowered.len() > 1
        && lowered[1..].chars().all(|ch| ch.is_ascii_digit()))
    .then_some(first_token.trim().to_string())
}

fn normalize_generated_task_reference(reference: &str) -> String {
    reference.trim().to_ascii_lowercase()
}

fn is_bootstrap_task(task: &CoordinationTask) -> bool {
    if task.kind != CoordinationTaskKind::GeneratedWriteTask {
        return false;
    }

    let title = task.title.to_ascii_lowercase();
    let prompt = task.prompt.to_ascii_lowercase();
    let write_scope = task
        .write_scope
        .iter()
        .map(|scope| scope.to_ascii_lowercase())
        .collect::<Vec<_>>();

    title.contains("bootstrap")
        || title.contains("scaffold")
        || prompt.contains("bootstrap")
        || prompt.contains("scaffold")
        || write_scope.iter().any(|scope| scope == "cargo.toml")
}
