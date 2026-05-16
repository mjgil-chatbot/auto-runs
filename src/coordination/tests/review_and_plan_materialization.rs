#[test]
fn expand_review_task_materializes_missing_file_from_dependency_artifact() {
    let fixture = CoordinationTestFixture::named("expansion-materialize-from-artifact");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let dependency_task = fixture.make_task_with_options(
        &run,
        "track-4",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("claude"),
        "Draft Track 4",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["docs/autonomous/design-doc.md".to_string()],
        vec!["docs/autonomous/prompts/track-4-sail-spark.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let review_task = fixture.make_task_with_options(
        &run,
        "review",
        None,
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["docs/autonomous/prompts".to_string()],
        Vec::new(),
        vec![dependency_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[dependency_task.clone(), review_task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: dependency_task.task_id.clone(),
        provider: Some("claude".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Created Track 4 artifact".to_string(),
        body: "Created docs/autonomous/prompts/track-4-sail-spark.md".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Created Track 4 artifact".to_string(),
            details: "Created docs/autonomous/prompts/track-4-sail-spark.md".to_string(),
            changed_files: vec!["docs/autonomous/prompts/track-4-sail-spark.md".to_string()],
            ..Default::default()
        },
    });
    fixture.write_artifact_materialized_files(
        &run.run_id,
        &dependency_task.task_id,
        &[(
            "docs/autonomous/prompts/track-4-sail-spark.md",
            "# Track 4: Sail / Spark\n\n## Stage 0 Validation\n\nPass.\n\n## Final Prompt\n\nPrompt body.\n\n## Locked GTFA\n\nspark-connect\n\n## Step-by-Step Solution\n\n1. Step.\n\n## Numbered Required Sources\n\n1. Source.\n\n## Stump Reports\n\n- claude\n- gpt-5.4\n",
        )],
    );

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "materialize missing file".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "materialize missing file".to_string(),
            details: "prompt file missing".to_string(),
            remaining_gaps: vec!["prompt file missing".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Write Track 4 Sail/Spark prompt artifact to disk".to_string(),
                provider: Some("claude".to_string()),
                prompt: "Write the prompt artifact to disk".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["artifacts/**".to_string()],
                write_scope: vec!["docs/autonomous/prompts/track-4-sail-spark.md".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "file exists".to_string(),
                    "sections are present".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let written = fs::read_to_string(
        fixture
            .workspace
            .path()
            .join("docs/autonomous/prompts/track-4-sail-spark.md"),
    )
    .unwrap();
    assert!(written.contains("# Track 4: Sail / Spark"));

    let tasks = load_tasks(&run.run_id).unwrap();
    let generated_write_children = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .filter(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .collect::<Vec<_>>();
    assert!(generated_write_children.is_empty());

    let follow_up = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(review_task.task_id.as_str()))
        .find(|task| task.kind == CoordinationTaskKind::ReviewArtifacts)
        .unwrap();
    assert_eq!(follow_up.dependencies, vec![review_task.task_id.clone()]);

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "review_fix_materialized"
            && event
                .message
                .contains("docs/autonomous/prompts/track-4-sail-spark.md")
    }));
}

#[test]
fn expand_review_task_materializes_track_alias_from_dependency_artifact() {
    let fixture = CoordinationTestFixture::named("expansion-materialize-alias");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let dependency_task = fixture.make_task_with_options(
        &run,
        "track-1",
        None,
        CoordinationTaskKind::GeneratedWriteTask,
        Some("claude"),
        "Draft Track 1",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["docs/autonomous/design-doc.md".to_string()],
        vec!["docs/autonomous/prompts/track-1-weiner-chou.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let review_task = fixture.make_task_with_options(
        &run,
        "review",
        None,
        CoordinationTaskKind::ReviewArtifacts,
        Some("codex"),
        "Review",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["docs/autonomous/prompts".to_string()],
        Vec::new(),
        vec![dependency_task.task_id.clone()],
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[dependency_task.clone(), review_task.clone()]);
    fixture.write_artifact(&TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: dependency_task.task_id.clone(),
        provider: Some("claude".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "Created Track 1 artifact".to_string(),
        body: "# Track 1\n\n## Stage 0 Validation\n\nPass.\n\n## Final Prompt\n\nPrompt body.\n\n## Locked GTFA\n\npinlater\n\n## Step-by-Step Solution\n\n1. Step.\n\n## Numbered Required Sources\n\n1. Source.\n\n## Stump Reports\n\n- claude\n- gpt-5.4\n".to_string(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::CompletedWithChanges,
            summary: "Created Track 1 artifact".to_string(),
            details: "# Track 1\n\n## Stage 0 Validation\n\nPass.\n\n## Final Prompt\n\nPrompt body.\n\n## Locked GTFA\n\npinlater\n\n## Step-by-Step Solution\n\n1. Step.\n\n## Numbered Required Sources\n\n1. Source.\n\n## Stump Reports\n\n- claude\n- gpt-5.4\n".to_string(),
            changed_files: vec!["docs/autonomous/prompts/track-1-weiner-chou.md".to_string()],
            ..Default::default()
        },
    });

    let artifact = TaskArtifact {
        run_id: run.run_id.clone(),
        task_id: review_task.task_id.clone(),
        provider: Some("codex".to_string()),
        task_category: None,
        fallback_from: None,
        summary: "materialize alias".to_string(),
        body: String::new(),
        completed_at: Utc::now(),
        report: TaskExecutionReport {
            outcome: CoordinationTaskOutcome::Partial,
            summary: "materialize alias".to_string(),
            details: "prompt file missing".to_string(),
            remaining_gaps: vec!["prompt file missing".to_string()],
            fix_tasks: vec![GeneratedTaskSpec {
                title: "Write Track 1 Pinterest prompt artifact to disk".to_string(),
                provider: Some("claude".to_string()),
                prompt: "Write the prompt artifact to disk".to_string(),
                kind: "write".to_string(),
                estimated_minutes: 15,
                depends_on_titles: Vec::new(),
                read_scope: vec!["artifacts/**".to_string()],
                write_scope: vec!["docs/autonomous/prompts/track-1-pinterest-infra.md".to_string()],
                allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
                acceptance_checks: vec![
                    "file exists".to_string(),
                    "sections are present".to_string(),
                ],
            }],
            ..Default::default()
        },
    };

    expand_review_task(&run.run_id, &review_task, &artifact).unwrap();

    let written = fs::read_to_string(
        fixture
            .workspace
            .path()
            .join("docs/autonomous/prompts/track-1-pinterest-infra.md"),
    )
    .unwrap();
    assert!(written.contains("# Track 1"));
}

#[test]
fn expand_plan_task_generates_children_and_rewires_materialization() {
    let fixture = CoordinationTestFixture::named("expansion-plan");
    let run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let materialize_task = fixture.make_task_with_options(
        &run,
        "materialize",
        None,
        CoordinationTaskKind::MaterializeDesignDoc,
        Some("codex"),
        "Materialize",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["artifacts/**".to_string()],
        vec!["docs/design.md".to_string()],
        vec![plan_task.task_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone(), materialize_task.clone()]);

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "Patch src/lib.rs",
                "provider": "codex",
                "prompt": "Implement the missing guard",
                "kind": "write",
                "read_scope": [],
                "write_scope": ["src/lib.rs/"],
                "allowed_tools": [],
                "acceptance_checks": [
                    "src/lib.rs contains the guard",
                    "no unrelated files change"
                ]
            },
            {
                "title": "Inspect docs",
                "provider": null,
                "prompt": "Inspect the generated docs",
                "kind": "read",
                "read_scope": [],
                "write_scope": [],
                "allowed_tools": [],
                "acceptance_checks": []
            }
        ]"#,
    )
    .unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let generated = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);

    let write_task = generated
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .unwrap();
    assert_eq!(
        write_task.allowed_tools,
        vec!["filesystem".to_string(), "edit".to_string()]
    );
    assert_eq!(write_task.read_scope, vec!["**".to_string()]);
    assert_eq!(write_task.write_scope, vec!["src/lib.rs".to_string()]);
    assert_eq!(write_task.dependencies, vec![plan_task.task_id.clone()]);

    let read_task = generated
        .iter()
        .find(|task| task.kind == CoordinationTaskKind::GeneratedReadTask)
        .unwrap();
    assert_eq!(
        read_task.allowed_tools,
        vec!["filesystem".to_string(), "search".to_string()]
    );
    assert_eq!(read_task.read_scope, vec!["**".to_string()]);
    assert!(read_task.write_scope.is_empty());
    assert_eq!(read_task.dependencies, vec![plan_task.task_id.clone()]);

    let rewired_materialize = tasks
        .iter()
        .find(|task| task.task_id == materialize_task.task_id)
        .unwrap();
    assert_eq!(rewired_materialize.dependencies.len(), 2);
    assert!(rewired_materialize
        .dependencies
        .iter()
        .all(|dependency| generated.iter().any(|task| &task.task_id == dependency)));
    assert_eq!(rewired_materialize.status, CoordinationTaskStatus::Blocked);
}

#[test]
fn expand_plan_task_rewires_seeded_review_child_to_generated_tasks() {
    let fixture = CoordinationTestFixture::named("expansion-plan-review");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    let review_task = fixture.make_task_with_options(
        &run,
        "review",
        Some(&plan_task.task_id),
        CoordinationTaskKind::ReviewArtifacts,
        Some("claude"),
        "Review artifacts against design doc",
        vec!["filesystem".to_string()],
        vec!["artifacts/**".to_string(), "docs/design.md".to_string()],
        vec![".".to_string()],
        vec![plan_task.task_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    let apply_task = fixture.make_task_with_options(
        &run,
        "apply",
        Some(&review_task.task_id),
        CoordinationTaskKind::ApplyApprovedEdits,
        Some("codex"),
        "Apply non-overlapping edit tasks",
        vec!["filesystem".to_string(), "edit".to_string()],
        vec!["**".to_string()],
        vec![".".to_string()],
        vec![review_task.task_id.clone()],
        CoordinationTaskStatus::Blocked,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone(), review_task.clone(), apply_task]);

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "Draft Track 1",
                "provider": "codex",
                "prompt": "Write track 1",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["docs/autonomous/prompts/01.md"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": [
                    "Track 1 exists",
                    "No unrelated files change"
                ]
            },
            {
                "title": "Draft Track 2",
                "provider": "gemini",
                "prompt": "Write track 2",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["docs/autonomous/prompts/02.md"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": [
                    "Track 2 exists",
                    "No unrelated files change"
                ]
            }
        ]"#,
    )
    .unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let generated = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .filter(|task| task.kind == CoordinationTaskKind::GeneratedWriteTask)
        .collect::<Vec<_>>();
    assert_eq!(generated.len(), 2);

    let rewired_review = tasks
        .iter()
        .find(|task| task.task_id == review_task.task_id)
        .unwrap();
    assert_eq!(rewired_review.dependencies.len(), generated.len());
    assert!(rewired_review
        .dependencies
        .iter()
        .all(|dependency| generated.iter().any(|task| &task.task_id == dependency)));
    assert_eq!(rewired_review.status, CoordinationTaskStatus::Blocked);
}

#[test]
fn design_implementation_bootstrap_write_task_blocks_other_generated_writes() {
    let fixture = CoordinationTestFixture::named("expansion-bootstrap-deps");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone()]);

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "Bootstrap the Rust workspace and implement vectorvfs-core",
                "prompt": "Bootstrap the Cargo workspace and create the core crate.",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["Cargo.toml", "crates/vectorvfs-core/src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["workspace boots", "core crate exists"]
            },
            {
                "title": "Implement the provider crate and Python perception bridge",
                "prompt": "Build the provider crate on top of the bootstrapped workspace.",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["crates/vectorvfs-provider/src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["provider crate exists", "bridge exists"]
            },
            {
                "title": "Build the Rust CLI commands and end-to-end parity tests",
                "prompt": "Add CLI commands and tests once the scaffold exists.",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["crates/vectorvfs-cli/src/main.rs", "tests/e2e.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["cli exists", "tests exist"]
            }
        ]"#,
    )
    .unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let bootstrap = tasks
        .iter()
        .find(|task| task.title.contains("Bootstrap the Rust workspace"))
        .unwrap();
    let provider = tasks
        .iter()
        .find(|task| task.title.contains("provider crate"))
        .unwrap();
    let cli = tasks
        .iter()
        .find(|task| task.title.contains("Rust CLI commands"))
        .unwrap();

    assert_eq!(bootstrap.dependencies, vec![plan_task.task_id.clone()]);
    assert_eq!(
        provider.dependencies,
        vec![plan_task.task_id.clone(), bootstrap.task_id.clone()]
    );
    assert_eq!(
        cli.dependencies,
        vec![plan_task.task_id.clone(), bootstrap.task_id.clone()]
    );
}

#[test]
fn design_implementation_without_bootstrap_keeps_generated_write_dependencies_flat() {
    let fixture = CoordinationTestFixture::named("expansion-no-bootstrap-deps");
    let run = fixture.make_run(AutonomousWorkflowMode::DesignImplementation);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan",
        vec!["filesystem".to_string(), "search".to_string()],
        vec!["**".to_string()],
        Vec::new(),
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone()]);

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "Freeze the v1 storage contract and generate golden fixtures",
                "prompt": "Define the storage contract and fixtures.",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["fixtures/storage.json"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["fixtures exist", "contract is documented"]
            },
            {
                "title": "Implement the provider crate and Python perception bridge",
                "prompt": "Build the provider crate.",
                "kind": "write",
                "read_scope": ["docs/design.md"],
                "write_scope": ["crates/vectorvfs-provider/src/lib.rs"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["provider crate exists", "bridge exists"]
            }
        ]"#,
    )
    .unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let generated = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(generated.len(), 2);
    assert!(generated
        .iter()
        .all(|task| task.dependencies == vec![plan_task.task_id.clone()]));
}

#[test]
fn lightweight_plan_generated_tasks_resolve_dependency_graph_order() {
    let fixture = CoordinationTestFixture::named("lightweight-generated-deps");
    let run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    let plan_task = fixture.make_task_with_options(
        &run,
        "plan",
        None,
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Split lightweight implementation tasks",
        vec![
            "filesystem".to_string(),
            "search".to_string(),
            "edit".to_string(),
        ],
        vec!["**".to_string()],
        vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        Vec::new(),
        CoordinationTaskStatus::Completed,
    );
    fixture.persist_run_with_tasks(&run, &[plan_task.clone()]);

    expand_plan_task(
        &run.run_id,
        &plan_task,
        r#"[
            {
                "title": "T1 static shell and styles",
                "prompt": "Create the shell.",
                "kind": "write",
                "estimated_minutes": 12,
                "depends_on_titles": [],
                "read_scope": ["docs/autonomous/lightweight-tasks.md"],
                "write_scope": ["index.html"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["shell exists", "layout is responsive"]
            },
            {
                "title": "T2 block parser module",
                "prompt": "Create the block parser.",
                "kind": "write",
                "estimated_minutes": 9,
                "depends_on_titles": [],
                "read_scope": ["docs/autonomous/lightweight-tasks.md"],
                "write_scope": ["src/markdown/blocks.js"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["parser exists", "block output is structured"]
            },
            {
                "title": "T3 app controller",
                "prompt": "Wire the controller after T1-T2 are available.",
                "kind": "write",
                "estimated_minutes": 14,
                "depends_on_titles": [],
                "read_scope": ["index.html", "src/markdown/blocks.js"],
                "write_scope": ["src/app.js"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["controller exists", "controller wires the parser"]
            },
            {
                "title": "T4 architecture doc refresh",
                "prompt": "Refresh the doc after implementation lands.",
                "kind": "write",
                "estimated_minutes": 8,
                "depends_on_titles": ["T1 static shell and styles", "T3 app controller"],
                "read_scope": ["index.html", "src/app.js"],
                "write_scope": ["docs/autonomous/lightweight.md"],
                "allowed_tools": ["filesystem", "edit"],
                "acceptance_checks": ["doc exists", "doc matches the implementation"]
            }
        ]"#,
    )
    .unwrap();

    let tasks = load_tasks(&run.run_id).unwrap();
    let generated = tasks
        .iter()
        .filter(|task| task.parent_task_id.as_deref() == Some(plan_task.task_id.as_str()))
        .collect::<Vec<_>>();

    let t1 = generated
        .iter()
        .find(|task| task.title == "T1 static shell and styles")
        .unwrap();
    let t2 = generated
        .iter()
        .find(|task| task.title == "T2 block parser module")
        .unwrap();
    let t3 = generated
        .iter()
        .find(|task| task.title == "T3 app controller")
        .unwrap();
    let t4 = generated
        .iter()
        .find(|task| task.title == "T4 architecture doc refresh")
        .unwrap();

    assert_eq!(t1.estimated_minutes, Some(12));
    assert_eq!(t2.estimated_minutes, Some(9));
    assert_eq!(t3.estimated_minutes, Some(14));
    assert_eq!(t4.estimated_minutes, Some(8));
    assert_eq!(t1.dependencies, vec![plan_task.task_id.clone()]);
    assert_eq!(t2.dependencies, vec![plan_task.task_id.clone()]);
    assert_eq!(
        t3.dependencies,
        vec![
            plan_task.task_id.clone(),
            t1.task_id.clone(),
            t2.task_id.clone()
        ]
    );
    assert_eq!(
        t4.dependencies,
        vec![
            plan_task.task_id.clone(),
            t1.task_id.clone(),
            t3.task_id.clone()
        ]
    );
}
