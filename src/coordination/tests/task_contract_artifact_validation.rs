fn generated_write_spec_json(title: &str, write_scope: &[&str]) -> String {
    serde_json::json!([{
        "title": title,
        "provider": "codex",
        "prompt": "Implement the requested change",
        "kind": "write",
        "estimated_minutes": 15,
        "depends_on_titles": [],
        "read_scope": ["src/lib.rs"],
        "write_scope": write_scope,
        "allowed_tools": ["filesystem", "edit"],
        "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
    }])
    .to_string()
}

#[test]
fn build_provider_task_artifact_plans_generated_tasks() {
    let fixture = CoordinationTestFixture::named("artifact-plan");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "planner",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan tasks",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let artifact = build_provider_task_artifact(
        &task,
        &generated_write_spec_json("Patch src/lib.rs", &["src/lib.rs"]),
    )
    .expect("plan artifact should build");

    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert!(artifact
        .summary
        .contains("Planned 1 generated task slices."));
    assert!(artifact
        .report
        .design_requirements_satisfied
        .iter()
        .any(|item| item.contains("machine-readable task slices")));
}

#[test]
fn typed_plan_contract_requires_typed_envelope_instead_of_legacy_array() {
    let contract = lookup_contract(CoordinationTaskKind::PlanFlatTasks).unwrap();
    let errors = validate_contract_response(
        contract,
        &generated_write_spec_json("Patch src/lib.rs", &["src/lib.rs"]),
    );
    assert!(!errors.is_empty());
    assert!(errors[0].contains("expected typed `plan_tasks` envelope"));
}

#[test]
fn build_provider_task_artifact_persists_typed_plan_contract_meta() {
    let fixture = CoordinationTestFixture::named("artifact-typed-plan");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let task = fixture.make_task(
        &run,
        "planner-typed",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan tasks",
        &["src/lib.rs"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let response = serde_json::json!({
        "outcome": "completed_with_changes",
        "summary": "Planned one focused task",
        "details": "One narrow write task is enough.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": ["Returned machine-readable task slices."],
        "validation_errors": [],
        "payload_kind": "plan_tasks",
        "contract_version": 1,
        "payload": {
            "generated_tasks": [{
                "title": "Patch src/lib.rs",
                "prompt": "Implement the requested change",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": ["src/lib.rs"],
                "write_scope": ["src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
            }]
        }
    })
    .to_string();

    let artifact =
        build_provider_task_artifact(&task, &response).expect("typed plan artifact should build");

    let generated = parse_generated_task_specs(&artifact.body, Some(&run)).expect("typed body should parse");
    assert_eq!(generated.len(), 1);
    assert_eq!(generated[0].estimated_minutes, 15);
    let meta = load_artifact_contract_meta(&run.run_id, &task.task_id)
        .expect("contract metadata should persist");
    assert_eq!(meta.payload_kind, TaskPayloadKind::PlanTasks);
    assert_eq!(meta.contract_version, 1);
}

#[test]
fn validate_contract_response_for_task_accepts_workspace_absolute_generated_scopes() {
    let fixture = CoordinationTestFixture::named("artifact-typed-plan-absolute-scope");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let task = fixture.make_task(
        &run,
        "planner-absolute",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan tasks",
        &["docs/task-list.md"],
        &["docs/task-list.md"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let absolute_task_list = format!("{}/docs/task-list.md", run.workspace_path);
    let response = serde_json::json!({
        "outcome": "completed_with_changes",
        "summary": "Planned one focused task",
        "details": "Uses an absolute workspace path that should be normalized.",
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": ["Returned machine-readable task slices."],
        "validation_errors": [],
        "payload_kind": "plan_tasks",
        "contract_version": 1,
        "payload": {
            "generated_tasks": [{
                "title": "Patch docs/task-list.md",
                "prompt": "Implement the requested change",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": ["src/lib.rs"],
                "write_scope": [absolute_task_list],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
            }]
        }
    })
    .to_string();

    let contract = lookup_contract(CoordinationTaskKind::PlanFlatTasks).unwrap();
    assert!(validate_contract_response(contract, &response)
        .iter()
        .any(|error| error.contains("absolute scope")));
    assert!(validate_contract_response_for_task(contract, &response, &task).is_empty());
}

#[test]
fn normalize_contract_response_for_task_rewrites_workspace_absolute_scopes() {
    let fixture = CoordinationTestFixture::named("artifact-typed-plan-normalize");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let task = fixture.make_task(
        &run,
        "planner-normalize",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan tasks",
        &["docs/task-list.md"],
        &["docs/task-list.md"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let absolute_task_list = format!("{}/docs/task-list.md", run.workspace_path);
    let response = serde_json::json!({
        "outcome": "completed_with_changes",
        "summary": "Planned one focused task",
        "details": "Uses an absolute workspace path that should be normalized.",
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": ["Returned machine-readable task slices."],
        "validation_errors": [],
        "payload_kind": "plan_tasks",
        "contract_version": 1,
        "payload": {
            "generated_tasks": [{
                "title": "Patch docs/task-list.md",
                "prompt": "Implement the requested change",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": [absolute_task_list],
                "write_scope": [absolute_task_list],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
            }]
        }
    })
    .to_string();

    let contract = lookup_contract(CoordinationTaskKind::PlanFlatTasks).unwrap();
    let normalized =
        normalize_contract_response_for_task(contract, &response, &task).expect("normalize");
    let envelope = parse_contract_envelope_from_response(&normalized).expect("typed envelope");
    let payload = plan_tasks_payload_from_envelope(&envelope).expect("payload");

    assert_eq!(payload.generated_tasks.len(), 1);
    assert_eq!(payload.generated_tasks[0].read_scope, vec!["docs/task-list.md"]);
    assert_eq!(payload.generated_tasks[0].write_scope, vec!["docs/task-list.md"]);
}

#[test]
fn lightweight_plan_contract_explicitly_requires_repo_relative_scopes() {
    let fixture = CoordinationTestFixture::named("lightweight-plan-relative-scopes");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task_with_options(
        &run,
        "task",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Bootstrap lightweight plan",
        vec!["filesystem".to_string(), "search".to_string(), "edit".to_string()],
        vec!["**".to_string()],
        vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Queued,
    );

    let contract = provider_response_contract(&task);
    assert!(contract.contains("repo-relative workspace paths"));
    assert!(contract.contains("never absolute paths"));
}

#[test]
fn build_provider_task_artifact_omits_changed_files_for_lightweight_typed_plan() {
    let fixture = CoordinationTestFixture::named("artifact-typed-plan-writeback");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    fixture.persist_run_with_tasks(&run, &[]);
    let task = fixture.make_task(
        &run,
        "planner-typed-writeback",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Bootstrap lightweight tasks",
        &["docs/autonomous/lightweight-tasks.md"],
        &["docs/autonomous/lightweight-tasks.md"],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let response = serde_json::json!({
        "outcome": "completed_with_changes",
        "summary": "Bootstrapped the task list and planned one slice",
        "details": "Created the initial task list and emitted one implementation task.",
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": ["Bootstrapped the coordination document."],
        "validation_errors": [],
        "payload_kind": "plan_tasks",
        "contract_version": 1,
        "payload": {
            "generated_tasks": [{
                "title": "Patch src/lib.rs",
                "prompt": "Implement the requested change",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": ["src/lib.rs"],
                "write_scope": ["src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
            }]
        }
    })
    .to_string();

    let artifact =
        build_provider_task_artifact(&task, &response).expect("typed plan artifact should build");

    assert_eq!(
        artifact.report.outcome,
        CoordinationTaskOutcome::CompletedWithChanges
    );
    assert!(artifact.report.changed_files.is_empty());
}

#[test]
fn build_provider_task_artifact_uses_design_doc_payload_markdown_body() {
    let fixture = CoordinationTestFixture::named("artifact-typed-design-doc");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignConsensus);
    let task = fixture.make_task(
        &run,
        "draft-typed",
        CoordinationTaskKind::DraftDesignDoc,
        Some("claude"),
        "Draft design doc",
        &["docs/autonomous/input.md"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );

    let response = serde_json::json!({
        "outcome": "completed_no_change",
        "summary": "Drafted the design document",
        "details": "Synthesized the design tradeoffs.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": ["Captured the requested architecture decisions."],
        "validation_errors": [],
        "payload_kind": "design_doc",
        "contract_version": 1,
        "payload": {
            "markdown": "# Runtime hardening\n\nDocument body."
        }
    })
    .to_string();

    let artifact = build_provider_task_artifact(&task, &response)
        .expect("typed design-doc artifact should build");

    assert_eq!(artifact.body, "# Runtime hardening\n\nDocument body.");
    let meta = load_artifact_contract_meta(&run.run_id, &task.task_id)
        .expect("design-doc contract metadata should persist");
    assert_eq!(meta.payload_kind, TaskPayloadKind::DesignDoc);
    assert_eq!(meta.contract_version, 1);
}

#[test]
fn typed_review_contract_requires_payload_envelope() {
    let contract = lookup_contract(CoordinationTaskKind::ReviewArtifacts).unwrap();
    let response = serde_json::json!({
        "outcome": "completed_no_change",
        "summary": "No issues found",
        "details": "The workspace is compliant.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": [],
        "fix_tasks": []
    })
    .to_string();

    let errors = validate_contract_response(contract, &response);
    assert!(!errors.is_empty());
    assert!(errors[0].contains("expected typed `review_findings` envelope"));
}

#[test]
fn normalize_review_completed_outcome_passes_contract_validation() {
    let contract = lookup_contract(CoordinationTaskKind::ReviewArtifacts).unwrap();
    let response = serde_json::json!({
        "outcome": "review_completed",
        "summary": "Reviewed files",
        "details": "No issues.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": [],
        "validation_errors": [],
        "payload_kind": "review_findings",
        "contract_version": 1,
        "payload": {
            "findings": [],
            "fix_tasks": []
        }
    })
    .to_string();

    let errors = validate_contract_response(contract, &response);
    assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
}

#[test]
fn normalize_completed_outcome_alias_passes_contract_validation() {
    let contract = lookup_contract(CoordinationTaskKind::PlanFlatTasks).unwrap();
    let response = serde_json::json!({
        "outcome": "completed",
        "summary": "Planned tasks",
        "details": "One task.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": [],
        "validation_errors": [],
        "payload_kind": "plan_tasks",
        "contract_version": 1,
        "payload": {
            "generated_tasks": [{
                "title": "Write code",
                "prompt": "Do work",
                "kind": "write",
                "estimated_minutes": 15,
                "depends_on_titles": [],
                "read_scope": [],
                "write_scope": ["src/lib.rs"],
                "allowed_tools": [],
                "acceptance_checks": ["cargo check succeeds", "tests cover the change"]
            }]
        }
    })
    .to_string();

    let errors = validate_contract_response(contract, &response);
    assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
}

#[test]
fn normalize_string_line_field_in_findings_passes_validation() {
    let contract = lookup_contract(CoordinationTaskKind::ReviewArtifacts).unwrap();
    let response = serde_json::json!({
        "outcome": "completed_no_change",
        "summary": "Found issues",
        "details": "Details.",
        "changed_files": [],
        "tests_run": [],
        "warnings": [],
        "blockers": [],
        "remaining_gaps": [],
        "design_requirements_satisfied": [],
        "validation_errors": [],
        "payload_kind": "review_findings",
        "contract_version": 1,
        "payload": {
            "findings": [{
                "severity": "high",
                "confidence": "90",
                "file": "src/lib.rs",
                "line": "23",
                "dimension": "error_handling",
                "description": "unwrap without error handling"
            }],
            "fix_tasks": []
        }
    })
    .to_string();

    let errors = validate_contract_response(contract, &response);
    assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);

    let envelope = parse_contract_envelope_from_response(&response).unwrap();
    let payload = review_findings_payload_from_envelope(&envelope).unwrap();
    assert_eq!(payload.findings.len(), 1);
    assert_eq!(payload.findings[0].line, 23);
}
