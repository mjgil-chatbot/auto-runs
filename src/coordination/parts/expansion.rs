use super::*;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;
pub fn try_materialize_review_fix_task(
    run_id: &str,
    review_task: &CoordinationTask,
    spec: &GeneratedTaskSpec,
) -> Result<Option<String>> {
    let kind = parse_generated_kind(&spec.kind)?;
    if kind != CoordinationTaskKind::GeneratedWriteTask {
        return Ok(None);
    }

    let missing_targets = spec
        .write_scope
        .iter()
        .map(|scope| normalize_scope(scope))
        .filter(|path| !Path::new(&review_task.cwd).join(path).is_file())
        .collect::<Vec<_>>();
    if missing_targets.len() != 1 {
        return Ok(None);
    }
    let output_path = &missing_targets[0];

    for dependency in &review_task.dependencies {
        let path = artifact_result_path(run_id, dependency);
        if !path.is_file() {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let dependency_artifact: TaskArtifact = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        let materialized_files = load_artifact_materialized_files(run_id, dependency);
        let Some(content) = materialized_artifact_content_for_path(
            &dependency_artifact,
            &materialized_files,
            output_path,
        ) else {
            continue;
        };

        let candidate = Path::new(&review_task.cwd).join(output_path);
        let parent = candidate
            .parent()
            .context("review fix output path missing parent")?;
        fs::create_dir_all(parent)?;
        fs::write(&candidate, content)
            .with_context(|| format!("failed to write {}", candidate.display()))?;
        append_event(
            run_id,
            "review_fix_materialized",
            Some(&review_task.task_id),
            &format!(
                "Materialized '{}' directly from dependency artifact {}.",
                output_path, dependency
            ),
        )?;
        return Ok(Some(output_path.clone()));
    }

    Ok(None)
}
pub fn rewire_follow_up_tasks(
    run_id: &str,
    source_task_id: &str,
    tasks: &mut [CoordinationTask],
    generated_ids: &[String],
) -> Result<()> {
    for task in tasks.iter_mut() {
        // Skip generated children — they are the newly created tasks (GeneratedReadTask,
        // GeneratedWriteTask) that already have the source as their parent.
        let is_generated_child = task.parent_task_id.as_deref() == Some(source_task_id)
            && is_generated_task_kind(&task.kind);
        if is_generated_child {
            continue;
        }
        // Also skip ReviewArtifacts tasks that appear in generated_ids — these are the
        // review tasks spawned by the plan that should keep their original dependencies.
        if task.kind == CoordinationTaskKind::ReviewArtifacts
            && generated_ids.iter().any(|id| id.as_str() == task.task_id)
        {
            continue;
        }
        if !task.dependencies.iter().any(|dep| dep == source_task_id) {
            continue;
        }
        match task.kind {
            CoordinationTaskKind::PlanFlatTasks => {
                task.dependencies = generated_ids.to_vec();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated downstream planning task dependencies to generated tasks.",
                )?;
            }
            CoordinationTaskKind::ReviewArtifacts => {
                task.dependencies = generated_ids
                    .iter()
                    .filter(|task_id| task_id.as_str() != task.task_id)
                    .cloned()
                    .collect();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated review task dependencies to generated tasks.",
                )?;
            }
            CoordinationTaskKind::MaterializeDesignDoc => {
                task.dependencies = generated_ids.to_vec();
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
                append_event(
                    run_id,
                    "task_rewired",
                    Some(&task.task_id),
                    "Updated materialization task dependencies to generated review tasks.",
                )?;
            }
            CoordinationTaskKind::ApplyApprovedEdits => {
                task.status = CoordinationTaskStatus::Blocked;
                task.updated_at = Utc::now();
                write_json_atomic(&task_path(run_id, &task.task_id), task)?;
            }
            _ => {}
        }
    }
    Ok(())
}
pub fn parse_generated_task_specs(
    body: &str,
    run: Option<&CoordinationRun>,
) -> Result<Vec<GeneratedTaskSpec>> {
    // In lightweight mode (GithubBranchFix, LightweightImplementation), skip the strict
    // typed-envelope contract and accept plain JSON arrays to avoid validation errors when
    // the provider returns loose JSON.
    let is_lightweight = run.map_or(false, |r| uses_lightweight_runtime_mode(&r.workflow_mode));
    if !is_lightweight {
        if lookup_contract(CoordinationTaskKind::PlanFlatTasks).is_some() {
            if let Ok(envelope) = parse_contract_envelope_from_response(body) {
                let payload = plan_tasks_payload_from_envelope(&envelope)?;
                return Ok(payload.generated_tasks);
            }
        }
    }
    let json = extract_first_json_array(body).context("missing JSON task array in plan output")?;
    serde_json::from_str(json).context("invalid generated task JSON")
}
pub fn validate_generated_task_specs(specs: &[GeneratedTaskSpec]) -> Result<()> {
    if specs.is_empty() {
        anyhow::bail!("generated task list is empty");
    }

    validate_generated_task_dependency_specs(specs)?;

    for spec in specs {
        if spec.title.trim().is_empty() || spec.prompt.trim().is_empty() {
            anyhow::bail!("generated task is missing a title or prompt");
        }
        if spec.provider.is_some() && normalize_provider_name(spec.provider.as_deref()).is_none() {
            anyhow::bail!("generated task '{}' uses an unknown provider", spec.title);
        }
        if !(MIN_TASK_ESTIMATED_MINUTES..=MAX_TASK_ESTIMATED_MINUTES)
            .contains(&spec.estimated_minutes)
        {
            anyhow::bail!(
                "generated task '{}' must set estimated_minutes between {} and {}",
                spec.title,
                MIN_TASK_ESTIMATED_MINUTES,
                MAX_TASK_ESTIMATED_MINUTES
            );
        }

        let kind = parse_generated_kind(&spec.kind)?;
        if kind == CoordinationTaskKind::GeneratedWriteTask {
            if spec.write_scope.is_empty() {
                anyhow::bail!(
                    "generated write task '{}' is missing write_scope",
                    spec.title
                );
            }
            if spec.acceptance_checks.len() < 2 {
                let normalized_checks = normalize_string_list(spec.acceptance_checks.clone());
                if normalized_checks.len() < 2 {
                    anyhow::bail!(
                        "generated write task '{}' is missing concrete acceptance checks",
                        spec.title
                    );
                }
            }
        }

        // Validate scope paths for traversal patterns.
        // Check for `..` path segments which could escape the workspace root,
        // and for absolute paths that could reference files outside the workspace.
        for scope in spec.read_scope.iter().chain(spec.write_scope.iter()) {
            if raw_scope_contains_parent_traversal(scope) {
                anyhow::bail!(
                    "generated task '{}' has scope '{}' containing '..' (parent-directory traversal)",
                    spec.title,
                    scope.trim()
                );
            }
            if raw_scope_is_absolute(scope) {
                anyhow::bail!(
                    "generated task '{}' has absolute scope '{}' which is not allowed",
                    spec.title,
                    scope.trim()
                );
            }

            let normalized = normalize_scope(scope);

            // Reject any path containing a parent-directory segment.
            if normalized.contains("..") {
                anyhow::bail!(
                    "generated task '{}' has normalized scope '{}' containing '..' (parent-directory traversal)",
                    spec.title,
                    normalized
                );
            }
        }
    }

    Ok(())
}
