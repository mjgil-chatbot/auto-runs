#[test]
fn accepts_repo_wide_generated_write_scope() {
    let specs = vec![GeneratedTaskSpec {
        title: "Repo-wide".to_string(),
        provider: Some("codex".to_string()),
        prompt: "Do everything".to_string(),
        kind: "write".to_string(),
        estimated_minutes: 15,
        depends_on_titles: Vec::new(),
        read_scope: vec!["**".to_string()],
        write_scope: vec!["**".to_string()],
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
    }];

    validate_generated_task_specs(&specs)
        .expect("repo-wide write_scope should be allowed for patch-level publish");
}

#[test]
fn rejects_generated_task_with_non_enum_provider() {
    let specs = vec![GeneratedTaskSpec {
        title: "Wrong provider".to_string(),
        provider: Some("gpt-5.4".to_string()),
        prompt: "Do one thing".to_string(),
        kind: "write".to_string(),
        estimated_minutes: 15,
        depends_on_titles: Vec::new(),
        read_scope: vec!["src/lib.rs".to_string()],
        write_scope: vec!["src/lib.rs".to_string()],
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
    }];

    let err = validate_generated_task_specs(&specs).expect_err("provider should be rejected");
    assert!(err.to_string().contains("unknown provider"));
}

#[test]
fn rejects_generated_task_with_unknown_dependency_reference() {
    let specs = vec![
        GeneratedTaskSpec {
            title: "T1 shell".to_string(),
            provider: Some("codex".to_string()),
            prompt: "Create the shell".to_string(),
            kind: "write".to_string(),
            estimated_minutes: 15,
            depends_on_titles: Vec::new(),
            read_scope: vec!["docs/design.md".to_string()],
            write_scope: vec!["index.html".to_string()],
            allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
            acceptance_checks: vec![
                "shell exists".to_string(),
                "layout is responsive".to_string(),
            ],
        },
        GeneratedTaskSpec {
            title: "T2 app".to_string(),
            provider: Some("codex".to_string()),
            prompt: "Wire the app".to_string(),
            kind: "write".to_string(),
            estimated_minutes: 15,
            depends_on_titles: vec!["T9 parser".to_string()],
            read_scope: vec!["index.html".to_string()],
            write_scope: vec!["src/app.js".to_string()],
            allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
            acceptance_checks: vec!["app exists".to_string(), "preview updates".to_string()],
        },
    ];

    let err = validate_generated_task_specs(&specs).expect_err("unknown dependency should fail");
    assert!(err.to_string().contains("unknown generated task reference"));
}

#[test]
fn rejects_generated_task_with_out_of_range_estimate() {
    let specs = vec![GeneratedTaskSpec {
        title: "Too long".to_string(),
        provider: Some("codex".to_string()),
        prompt: "Do one thing".to_string(),
        kind: "write".to_string(),
        estimated_minutes: 0,
        depends_on_titles: Vec::new(),
        read_scope: vec!["src/lib.rs".to_string()],
        write_scope: vec!["src/lib.rs".to_string()],
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
    }];

    let err = validate_generated_task_specs(&specs).expect_err("estimate should be rejected");
    assert!(err.to_string().contains("estimated_minutes"));
}

#[test]
fn rejects_generated_task_with_absolute_scope_input() {
    let specs = vec![GeneratedTaskSpec {
        title: "Absolute scope".to_string(),
        provider: Some("codex".to_string()),
        prompt: "Do one thing".to_string(),
        kind: "write".to_string(),
        estimated_minutes: 15,
        depends_on_titles: Vec::new(),
        read_scope: vec!["src/lib.rs".to_string()],
        write_scope: vec!["/home/developer/git/chatbot/chatbot-cli/src/lib.rs".to_string()],
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
    }];

    let err = validate_generated_task_specs(&specs).expect_err("absolute scope should be rejected");
    assert!(err.to_string().contains("absolute scope"));
}

#[test]
fn extract_first_json_array_skips_prose_arrays_before_task_list() {
    let body = r#"Based on my analysis:
- Files to modify: build.sh
- Write scope: ["build.sh"]

```json
[
  {
    "title": "Fix build script",
    "prompt": "Edit build.sh",
    "kind": "write",
    "estimated_minutes": 5,
    "depends_on_titles": [],
    "read_scope": ["build.sh"],
    "write_scope": ["build.sh"],
    "allowed_tools": ["Read", "Edit"],
    "acceptance_checks": ["build.sh updated", "git diff shows the intended change"]
  }
]
```"#;

    let parsed: Vec<GeneratedTaskSpec> =
        serde_json::from_str(extract_first_json_array(body).expect("task array should be found"))
            .expect("task array should parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].title, "Fix build script");
    assert_eq!(parsed[0].write_scope, vec!["build.sh".to_string()]);
}

#[test]
fn lightweight_implementation_generated_specs_may_still_declare_task_list_scope() {
    let specs = vec![GeneratedTaskSpec {
        title: "Mark checklist progress".to_string(),
        provider: Some("codex".to_string()),
        prompt: "Inspect checklist progress for review.".to_string(),
        kind: "write".to_string(),
        estimated_minutes: 15,
        depends_on_titles: Vec::new(),
        read_scope: vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        write_scope: vec!["docs/autonomous/lightweight-tasks.md".to_string()],
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
    }];

    validate_generated_task_specs(&specs)
        .expect("spec validation does not enforce lightweight task-list ownership");
}
