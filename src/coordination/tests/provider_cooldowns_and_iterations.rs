#[test]
fn build_provider_query_appends_dependency_artifacts_and_validator_contract() {
    let fixture = CoordinationTestFixture::named("provider-query");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let dependency = fixture.make_task_with_options(
        &run,
        "dependency",
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Dependency",
        vec!["filesystem".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        vec![dependency.task_id.clone()],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, &[dependency.clone(), task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: dependency.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "surveyed".to_string(),
        body: "Dependency details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "surveyed".to_string(),
            details: "Dependency details".to_string(),
            ..Default::default()
        },
    });

    let query = build_provider_query(&task).unwrap();
    assert!(query.contains("Prompt for Write task"));
    assert!(query.contains("Dependency artifacts:"));
    assert!(query.contains("Dependency details"));
    assert!(query.contains("validate_autonomous_task_report.py"));
    assert!(query.contains("--kind standard"));
    assert!(query.contains("--workspace-root"));
}

#[test]
fn build_provider_query_ignores_dependency_artifacts_from_other_generations() {
    let fixture = CoordinationTestFixture::named("provider-query-generation");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut dependency = fixture.make_task_with_options(
        &run,
        "dependency",
        None,
        CoordinationTaskKind::SurveyWorkspace,
        None,
        "Dependency",
        vec!["filesystem".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    dependency.execution_generation = 1;
    let mut task = fixture.make_task_with_options(
        &run,
        "write-task",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("codex"),
        "Write task",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        vec![dependency.task_id.clone()],
        CoordinationTaskStatus::Queued,
    );
    task.execution_generation = 2;
    fixture.persist_run_with_tasks(&run, &[dependency.clone(), task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: dependency.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "surveyed".to_string(),
        body: "stale dependency details".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedNoChange,
            summary: "surveyed".to_string(),
            details: "stale dependency details".to_string(),
            ..Default::default()
        },
    });

    let query = build_provider_query(&task).unwrap();
    assert!(query.contains("Prompt for Write task"));
    assert!(!query.contains("stale dependency details"));
    assert!(query.contains("validate_autonomous_task_report.py"));
}

#[test]
fn requeue_task_for_provider_cooldown_maps_quota_errors_to_retries() {
    let fixture = CoordinationTestFixture::named("provider-error");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task_with_options(
        &run,
        "provider-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("gemini"),
        "Provider task",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(77);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "gemini".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    let requeued = requeue_task_for_provider_cooldown(
        &run.run_id,
        &task.task_id,
        77,
        "gemini",
        "Gemini command failed.\nTerminalQuotaError: You have exhausted your capacity on this model. Your quota will reset after 21h1m15s.\nretryDelayMs: 75675110.700098,\nreason: 'QUOTA_EXHAUSTED'",
    )
    .unwrap();

    assert!(requeued);
    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.assigned_provider.as_deref(), Some("claude"));

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(
        updated_run.provider_cooldowns["gemini"].reason,
        "Gemini API quota exhausted."
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "provider_cooldown"));
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event.message.contains("Requeued Provider task via claude")
    }));
}

#[test]
fn requeue_task_for_provider_cooldown_maps_gemini_docker_capacity_errors_to_retries() {
    let fixture = CoordinationTestFixture::named("provider-error-gemini-docker");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task_with_options(
        &run,
        "provider-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("gemini"),
        "Provider task",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(77);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "gemini".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    let requeued = requeue_task_for_provider_cooldown(
        &run.run_id,
        &task.task_id,
        77,
        "gemini",
        "Gemini command failed.\nRecent stderr output:\nstatus: 429\nstatusText: 'Too Many Requests'\nAttempt 5 failed with status 429. Retrying with backoff...\n{\"error\":{\"status\":\"RESOURCE_EXHAUSTED\",\"details\":[{\"reason\":\"MODEL_CAPACITY_EXHAUSTED\"}]}}",
    )
    .unwrap();

    assert!(requeued);
    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.assigned_provider.as_deref(), Some("claude"));

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(
        updated_run.provider_cooldowns["gemini"].reason,
        "Gemini API quota exhausted."
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());
}

#[test]
fn requeue_task_for_provider_cooldown_maps_glm_rate_limits_to_retries() {
    let fixture = CoordinationTestFixture::named("provider-error-glm");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task_with_options(
        &run,
        "provider-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("glm"),
        "Provider task",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(77);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "glm".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    let requeued = requeue_task_for_provider_cooldown(
        &run.run_id,
        &task.task_id,
        77,
        "glm",
        "Claude command failed: API Error: 429 {\"error\":{\"code\":\"1308\",\"message\":\"已达到 5 小时的使用上限。您的限额将在 2099-12-31 23:59:59 重置。\"},\"request_id\":\"20260408015107f96597ccc364431a\"}",
    )
    .unwrap();

    assert!(requeued);
    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.attempt_count, task.attempt_count.saturating_sub(1));

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(
        updated_run.provider_cooldowns["glm"].reason,
        "GLM API quota exhausted."
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "provider_cooldown"
            && event.message.contains("Provider glm unavailable until")
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event.message.contains("Provider glm unavailable until")
    }));
}

#[test]
fn requeue_task_for_provider_cooldown_maps_auth_failures_to_fallback_retries() {
    let fixture = CoordinationTestFixture::named("provider-auth-error");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task_with_options(
        &run,
        "provider-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("claude"),
        "Provider task",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(77);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "claude".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    let requeued = requeue_task_for_provider_cooldown(
        &run.run_id,
        &task.task_id,
        77,
        "claude",
        "Claude command failed: Failed to authenticate. API Error: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\",\"message\":\"Invalid authentication credentials\"}}",
    )
    .unwrap();

    assert!(requeued);
    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);
    assert_eq!(queued.claimed_by, None);
    assert_eq!(queued.assigned_provider.as_deref(), Some("codex"));

    let updated_run = load_run(&run.run_id).unwrap();
    assert_eq!(
        updated_run.provider_cooldowns["claude"].reason,
        "Claude authentication failed."
    );
    assert!(!claim_path(&run.run_id, &task.task_id).exists());

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "provider_cooldown"));
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event.message.contains("Requeued Provider task via codex")
    }));
}

#[test]
fn fail_task_maps_glm_rate_limits_to_provider_cooldown_before_generic_retry() {
    let fixture = CoordinationTestFixture::named("provider-error-glm-fail");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let mut task = fixture.make_task_with_options(
        &run,
        "provider-task",
        None,
        CoordinationTaskKind::GeneratedReadTask,
        Some("glm"),
        "Provider task",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Running,
    );
    task.claimed_by = Some(77);
    
    fixture.persist_run_with_tasks(&run, &[task.clone()]);
    assert!(create_claim_file(&TaskClaim {
        run_id: run.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: "glm".to_string(),
        instance_pid: 77,
        launcher_pid: Some(77),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());

    fail_task(
        &run.run_id,
        &task.task_id,
        77,
        "Claude command failed: API Error: 429 {\"error\":{\"code\":\"1308\",\"message\":\"已达到 5 小时的使用上限。您的限额将在 2099-12-31 23:59:59 重置。\"},\"request_id\":\"20260408015107f96597ccc364431a\"}",
    )
    .unwrap();

    let queued = load_task(&run.run_id, &task.task_id).unwrap();
    assert_eq!(queued.status, CoordinationTaskStatus::Queued);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "provider_cooldown"));
    assert!(events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event.message.contains("Provider glm unavailable until")
    }));
    assert!(!events.iter().any(|event| {
        event.event_type == "task_retry_scheduled"
            && event
                .message
                .contains("Retry scheduled: Provider task after attempt")
    }));
}

#[test]
fn expand_ralph_iteration_queues_next_iteration_when_incomplete() {
    let fixture = CoordinationTestFixture::named("ralph-continue");
    let mut run = fixture.make_run(AutonomousWorkflowMode::RalphLoop);
    run.max_iterations = 3;
    let tasks = seed_initial_tasks(&run);
    run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
    fixture.persist_run_with_tasks(&run, &tasks);

    let initial_plan = tasks
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphPlan)
        .unwrap()
        .clone();
    let verify_task = tasks
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphVerify)
        .unwrap()
        .clone();

    expand_ralph_iteration(
        &run.run_id,
        &verify_task,
        "{\"complete\":false,\"reason\":\"missing coverage\"}",
    )
    .unwrap();

    let loaded_run = load_run(&run.run_id).unwrap();
    assert_eq!(loaded_run.task_ids.len(), run.task_ids.len() + 3);

    let generated = load_tasks(&run.run_id)
        .unwrap()
        .into_iter()
        .filter(|task| task.iteration == Some(2))
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 3);

    let next_plan = generated
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphPlan)
        .unwrap();
    let next_execute = generated
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphExecute)
        .unwrap();
    let next_verify = generated
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphVerify)
        .unwrap();

    assert_eq!(
        next_plan.dependencies,
        vec![initial_plan.task_id.clone(), verify_task.task_id.clone()]
    );
    assert_eq!(next_execute.dependencies, vec![next_plan.task_id.clone()]);
    assert_eq!(
        next_verify.dependencies,
        vec![next_plan.task_id.clone(), next_execute.task_id.clone()]
    );
}

#[test]
fn expand_ralph_iteration_records_verified_completion_without_new_tasks() {
    let fixture = CoordinationTestFixture::named("ralph-complete");
    let mut run = fixture.make_run(AutonomousWorkflowMode::RalphLoop);
    run.max_iterations = 3;
    let tasks = seed_initial_tasks(&run);
    run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
    fixture.persist_run_with_tasks(&run, &tasks);

    let verify_task = tasks
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::RalphVerify)
        .unwrap()
        .clone();

    expand_ralph_iteration(
        &run.run_id,
        &verify_task,
        "{\"complete\":true,\"reason\":\"all criteria satisfied\"}",
    )
    .unwrap();

    let reloaded_tasks = load_tasks(&run.run_id).unwrap();
    assert_eq!(reloaded_tasks.len(), tasks.len());
    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "ralph_verified" && event.message.contains("all criteria satisfied")
    }));
}

#[test]
fn expand_review_task_inherits_follow_up_scope_and_normalizes_fix_scope() {
    let fixture = CoordinationTestFixture::named("expansion-scopes");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let review_task = fixture.make_task_with_options(
        &run,
        "review",
        None,
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["src/lib.rs".to_string(), "docs/design.md".to_string()],
        vec!["docs/review-notes.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let apply_task = fixture.make_task_with_options(
        &run,
        "apply",
        None,
        CoordinationTaskKind::ApplyApprovedEdits,
        Some("codex"),
        "Apply",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["src/lib.rs".to_string()],
        vec!["src/lib.rs".to_string()],
        vec![review_task.task_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[review_task.clone(), apply_task.clone()]);

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "needs patch".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "needs patch".to_string(),
            details: "follow up".to_string(),
            blockers: vec!["patch required".to_string()],
            remaining_gaps: vec!["re-review after patch".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Patch src/lib.rs".to_string(),
                provider: Some("codex".to_string()),
                prompt: "Patch the file".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["src/lib.rs".to_string()],
                write_scope: vec!["src/lib.rs/".to_string()],
                allowed_tools: Vec::new(),
                acceptance_checks: vec![
                    "src/lib.rs compiles".to_string(),
                    "the missing guard exists".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let fix_task = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .unwrap();
    assert_eq!(fix_task.write_scope, vec!["src/lib.rs".to_string()]);
    assert_eq!(fix_task.cwd, run.workspace_path);
    assert_eq!(
        fix_task.allowed_tools,
        vec!["filesystem".to_string(), "edit".to_string()]
    );

    let follow_up = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .unwrap();
    assert_eq!(follow_up.read_scope, review_task.read_scope);
    assert_eq!(follow_up.write_scope, review_task.write_scope);
    assert_eq!(follow_up.acceptance_checks, review_task.acceptance_checks);
    assert_eq!(follow_up.dependencies, vec![fix_task.task_id.clone()]);

    let rewired_apply = tasks
        .iter()
        .find(|task| task.task_id == apply_task.task_id)
        .unwrap();
    assert_eq!(rewired_apply.dependencies, vec![follow_up.task_id.clone()]);
}
