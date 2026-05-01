use super::*;

pub fn complete_task(
    run_id: &str,
    task_id: &str,
    instance_pid: u32,
    artifact: TaskArtifact,
) -> Result<()> {
    let task = load_task(run_id, task_id)?;
    let materialized_files = capture_artifact_materialized_files(&task, &artifact);
    complete_task_with_materialized_files(
        run_id,
        task_id,
        instance_pid,
        artifact,
        materialized_files,
    )
}

pub fn complete_task_with_materialized_files(
    run_id: &str,
    task_id: &str,
    instance_pid: u32,
    mut artifact: TaskArtifact,
    materialized_files: Vec<MaterializedArtifactFile>,
) -> Result<()> {
    let mut mutation = begin_mutation_batch(
        run_id,
        Some(task_id),
        CoordinationMutationKind::CompleteTask,
        "Completing task artifact, task state, and follow-on recovery metadata.",
    )?;
    let mut run = load_run(run_id)?;
    let claim = load_claim(run_id, task_id)?;
    if claim.instance_pid != instance_pid {
        anyhow::bail!(
            "task {} for run {} is owned by PID {}, not {}",
            task_id,
            run_id,
            claim.instance_pid,
            instance_pid
        );
    }

    let mut task = load_task(run_id, task_id)?;
    let lightweight_skips_artifact_acceptance = LightweightWorkflowBehavior::for_run(&run)
        .map(|behavior| behavior.skips_artifact_acceptance())
        .unwrap_or(false);
    let lightweight_persists_materialized_files = LightweightWorkflowBehavior::for_run(&run)
        .map(|behavior| behavior.persists_materialized_files())
        .unwrap_or(true);
    if artifact.provider.is_none() {
        artifact.provider = task
            .assigned_provider
            .clone()
            .or_else(|| Some(claim.provider.clone()));
    }
    if artifact.task_category.is_none() {
        artifact.task_category = task.task_category;
    }
    if artifact.fallback_from.is_none() {
        artifact.fallback_from = task.provider_hint.clone();
    }
    let task_provider = task
        .assigned_provider
        .clone()
        .unwrap_or_else(|| claim.provider.clone());
    if lightweight_skips_artifact_acceptance {
        artifact.report.validation_errors =
            normalize_string_list(std::mem::take(&mut artifact.report.validation_errors));
        artifact
            .report
            .validation_errors
            .extend(apply_lightweight_completion_rails(
                &run,
                &task,
                &artifact,
                &materialized_files,
            ));
    } else {
        artifact.report.validation_errors = validate_task_artifact_with_materialized_files(
            run_id,
            &task,
            &artifact,
            &materialized_files,
        )?;
        artifact
            .report
            .validation_errors
            .extend(apply_completion_rails(
                &run,
                &task,
                &artifact,
                &materialized_files,
                &task_provider,
            )?);
    }

    let validation_failed = !artifact.report.validation_errors.is_empty();
    let explicit_failure = artifact.report.outcome == CoordinationTaskOutcome::Failed;
    let blocked_write_failure = task_mutates_workspace(&task)
        && artifact.report.outcome == CoordinationTaskOutcome::Blocked;
    let outcome_label = artifact.report.outcome.as_str();

    if !(validation_failed || explicit_failure || blocked_write_failure) {
        if update_lightweight_task_doc_after_review(&mut run, &task, &artifact, &materialized_files)
            .is_some()
        {
            run.updated_at = Utc::now();
            write_json_atomic(&run_path(run_id), &run)?;
            index_run(&run);
        }
        match task.kind {
            CoordinationTaskKind::PlanFlatTasks => expand_plan_task(run_id, &task, &artifact.body)?,
            CoordinationTaskKind::PlanCodeReview => {
                expand_code_review_plan(run_id, &task, &artifact.body)?
            }
            CoordinationTaskKind::ReviewArtifacts => expand_review_task(run_id, &task, &artifact)?,
            CoordinationTaskKind::RalphVerify => {
                expand_ralph_iteration(run_id, &task, &artifact.body)?
            }
            CoordinationTaskKind::PerfParse => {
                expand_perf_parse_task(run_id, &task, &artifact.body)?
            }
            CoordinationTaskKind::PerfEvaluate => {
                expand_perf_evaluate_task(run_id, &task, &artifact.body)?
            }
            _ => {}
        }
    }

    fs::create_dir_all(artifact_dir(run_id, task_id))?;
    write_json_atomic(&artifact_result_path(run_id, task_id), &artifact)?;
    fs::write(artifact_notes_path(run_id, task_id), &artifact.body)?;
    let publish_requires_materialized_files = uses_lightweight_runtime_mode(&run.workflow_mode)
        && task.kind == CoordinationTaskKind::GeneratedWriteTask;
    if !lightweight_persists_materialized_files && !publish_requires_materialized_files {
        let _ = fs::remove_file(artifact_materialized_files_path(run_id, task_id));
    } else {
        persist_artifact_materialized_files(run_id, task_id, &materialized_files)?;
    }

    if !(validation_failed || explicit_failure || blocked_write_failure)
        && requeue_github_branch_fix_poll_completion(run_id, &mut task, &task_provider, &artifact)?
    {
        index_provider_usage(
            run_id,
            task_id,
            &task_provider,
            task.task_category.as_ref(),
            &artifact.report.outcome,
            Some(&task.created_at),
            &Utc::now(),
            provider_cost_tier_for(&task_provider).as_deref(),
        );
        return Ok(());
    }

    let failure_message = if validation_failed && !blocked_write_failure {
        format!(
            "{} [{}]. Acceptance checks failed: {}",
            artifact.summary,
            outcome_label,
            artifact.report.validation_errors.join(" | ")
        )
    } else {
        format!("{} [{}]", artifact.summary, outcome_label)
    };
    if validation_failed || explicit_failure || blocked_write_failure {
        artifact.report.failure_kind = Some(infer_failure_kind(&failure_message));
        artifact.report.recovery_action = Some(if blocked_write_failure {
            TaskRecoveryAction::ManualIntervention
        } else {
            recovery_action_for_failure_kind(infer_failure_kind(&failure_message))
        });
        write_json_atomic(&artifact_result_path(run_id, task_id), &artifact)?;
    }

    if artifact.task_category.is_some() {
        task.task_category = artifact.task_category;
    }
    if artifact.fallback_from.is_some() && task.provider_hint.is_none() {
        task.provider_hint = artifact.fallback_from.clone();
    }

    if (validation_failed || explicit_failure || blocked_write_failure)
        && recover_perf_idea_failure(
            run_id,
            &mut task,
            instance_pid,
            &task_provider,
            &failure_message,
        )?
    {
        return Ok(());
    }

    if validation_failed || explicit_failure || blocked_write_failure {
        if blocked_write_failure {
            finalize_blocked_write_task(
                run_id,
                &mut task,
                instance_pid,
                &task_provider,
                &failure_message,
            )?;
        } else {
            finalize_failed_task(run_id, &mut task, instance_pid, &failure_message)?;
        }
        index_provider_usage(
            run_id,
            task_id,
            &task_provider,
            task.task_category.as_ref(),
            &artifact.report.outcome,
            Some(&task.created_at),
            &Utc::now(),
            provider_cost_tier_for(&task_provider).as_deref(),
        );
    } else {
        persist_design_consensus_draft_doc_copy(&task, &artifact)?;
        write_materialized_files_to_workspace(&task, &materialized_files)?;
        task.status = CoordinationTaskStatus::Completed;
        task.claimed_by = None;
        task.artifact_summary = Some(format!("{} [{}]", artifact.summary, outcome_label));
        if uses_lightweight_runtime_mode(&run.workflow_mode)
            && task.kind == CoordinationTaskKind::GeneratedWriteTask
        {
            task.failure_kind = Some(TaskFailureKind::Publish);
            task.recovery_action = Some(TaskRecoveryAction::RetryPublish);
        } else {
            task.failure_kind = None;
            task.recovery_action = None;
        }
        task.updated_at = Utc::now();
        write_json_atomic(&task_path(run_id, task_id), &task)?;
        index_task(&task);
        let _ = fs::remove_file(claim_path(run_id, task_id));
        append_event_with_provider(
            run_id,
            "task_completed",
            Some(task_id),
            Some(&task_provider),
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Completed: {} [{}]. {}{}",
                task.title,
                outcome_label,
                artifact.summary,
                if task.recovery_action == Some(TaskRecoveryAction::RetryPublish) {
                    " Git publish deferred to reconciler."
                } else {
                    ""
                }
            ),
        )?;
        let _ = maybe_create_review_approval_interrupt(run_id, &task, &artifact);
        index_provider_usage(
            run_id,
            task_id,
            &task_provider,
            task.task_category.as_ref(),
            &artifact.report.outcome,
            Some(&task.created_at),
            &Utc::now(),
            provider_cost_tier_for(&task_provider).as_deref(),
        );
        sync_run_summary_after_task_update(run_id, task.updated_at)?;
        if task.recovery_action == Some(TaskRecoveryAction::RetryPublish) {
            let _ = reconcile_run_state(run_id);
        }
    }

    if validation_failed {
        append_event_with_provider(
            run_id,
            "task_validation_failed",
            Some(task_id),
            Some(&task_provider),
            task.task_category,
            task.provider_hint.as_deref(),
            &format!(
                "Acceptance checks failed for {}: {}",
                task.title,
                artifact.report.validation_errors.join(" | ")
            ),
        )?;
    }
    complete_mutation_batch(&mut mutation)?;
    Ok(())
}

fn short_git_commit_display(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}
