    #[test]
    fn scope_globs_normalizes_backslash_patterns() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-bslash-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src/nested")).unwrap();
        fs::write(workspace.join("src/nested/mod.rs"), "fn nested() {}\n").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Globs(vec!["src\\nested\\*.rs".to_string()]),
        )
        .unwrap();

        assert_eq!(files, vec!["src/nested/mod.rs".to_string()]);
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Glob matching: complex backtracking patterns ─────────────────

    #[test]
    fn glob_star_backtracking_multiple_candidates() {
        // Pattern "a*b" against "aXYZb" — star must backtrack
        assert!(scope::glob_matches_path_test("a*b", "aXYZb"));
        // "a*b" should not match "aXYZc"
        assert!(!scope::glob_matches_path_test("a*b", "aXYZc"));
    }

    #[test]
    fn glob_multiple_stars_in_component() {
        assert!(scope::glob_matches_path_test("src/*test*.rs", "src/my_test_file.rs"));
        assert!(!scope::glob_matches_path_test("src/*test*.rs", "src/my_file.rs"));
    }

    #[test]
    fn glob_question_mark_exact_length() {
        assert!(scope::glob_matches_path_test("src/???.rs", "src/abc.rs"));
        assert!(!scope::glob_matches_path_test("src/???.rs", "src/ab.rs"));
        assert!(!scope::glob_matches_path_test("src/???.rs", "src/abcd.rs"));
    }

    #[test]
    fn glob_consecutive_double_stars() {
        // "**/**/*.rs" — consecutive double-stars still match
        assert!(scope::glob_matches_path_test("**/**/*.rs", "src/nested/deep/mod.rs"));
        assert!(scope::glob_matches_path_test("**/**/*.rs", "mod.rs"));
    }

    // ── dimensions_for_chunk: unwrap detection ───────────────────────

    #[test]
    fn dimensions_detects_unwrap_as_error_handling() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-unwrap-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("risky.rs"),
            "fn main() { let x = foo().unwrap(); }\n",
        )
        .unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "risky.rs".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, None).unwrap();
        assert!(dims.contains(&"error_handling".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── dimensions_for_chunk: whitespace-only selected dims ignored ──

    #[test]
    fn dimensions_whitespace_selected_dims_ignored() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-ws-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("plain.rs"), "fn plain() {}\n").unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "plain.rs".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        let selected = vec!["  ".to_string(), "".to_string()];
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, Some(&selected)).unwrap();
        // Whitespace-only selected dims should be filtered, triggering code_quality default
        assert!(dims.contains(&"code_quality".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── render_review_document: verification section with no automated checks ──

    #[test]
    fn render_review_document_empty_verification() {
        let metadata = ReviewTaskMetadata {
            scope_description: "workspace".to_string(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: None,
            output_path: None,
        };
        let doc = render_review_document(
            "test-proj",
            NaiveDate::from_ymd_opt(2026, 4, 5).unwrap(),
            "workspace",
            &metadata,
            &[],
        );
        assert!(doc.contains("No automated verification metadata was recorded"));
    }

    // ── render_review_document: whitespace-only fix_suggestion is not rendered ──

    #[test]
    fn render_review_document_whitespace_fix_suggestion_omitted() {
        let metadata = ReviewTaskMetadata {
            scope_description: "workspace".to_string(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: None,
            output_path: None,
        };
        let doc = render_review_document(
            "test-proj",
            NaiveDate::from_ymd_opt(2026, 4, 5).unwrap(),
            "workspace",
            &metadata,
            &[ReviewArtifactResponse {
                report: TaskExecutionReport::default(),
                findings: vec![ReviewFinding {
                    severity: ReviewSeverity::High,
                    confidence: "high".to_string(),
                    file: "src/a.rs".to_string(),
                    line: 1,
                    dimension: "architecture".to_string(),
                    description: "Issue".to_string(),
                    fix_suggestion: "   ".to_string(), // whitespace only
                }],
            }],
        );
        assert!(!doc.contains("Suggested fix:"));
    }

    // ── build_chunk_prompt: no automated checks ──────────────────────

    #[test]
    fn build_chunk_prompt_no_checks_shows_fallback_message() {
        let metadata = ReviewTaskMetadata {
            scope_description: "test".to_string(),
            dimensions: vec!["architecture".to_string()],
            chunk: vec![ReviewChunkTarget {
                path: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
            }],
            automated_checks: Vec::new(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: None,
            output_path: None,
        };
        let prompt = chunking::build_chunk_prompt(&metadata);
        assert!(prompt.contains("No automated checks were recorded"));
    }

    // ── describe_scope: multiple globs ───────────────────────────────

    #[test]
    fn describe_scope_multiple_globs() {
        let scope = ReviewScope::Globs(vec!["src/**/*.rs".into(), "tests/*.rs".into()]);
        let desc = describe_scope(&scope);
        assert!(desc.contains("src/**/*.rs"));
        assert!(desc.contains("tests/*.rs"));
    }

    // ── scope resolution: Globs with empty pattern skips silently ────

    #[test]
    fn scope_globs_empty_pattern_is_skipped() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-empty-glob-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Globs(vec!["".to_string(), "src/*.rs".to_string()]),
        )
        .unwrap();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
        // Empty pattern should not produce a warning
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── validate_review_response with valid findings ─────────────────

    // ── Chunking: MAX_REVIEW_CHUNK_FILES boundary ──────────────────

    #[test]
    fn chunk_builder_splits_when_file_count_exceeds_max() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-maxfiles-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        // Create MAX_REVIEW_CHUNK_FILES + 5 tiny files — each is 1 line so
        // the line-count limit is never hit; the file-count boundary should
        // still cause a split.
        let file_count = MAX_REVIEW_CHUNK_FILES + 5;
        let mut files = Vec::new();
        for i in 0..file_count {
            let name = format!("f{}.rs", i);
            fs::write(workspace.join(&name), "fn f() {}\n").unwrap();
            files.push(name);
        }

        let chunks = chunking::build_chunks(&workspace, &files).unwrap();
        assert!(
            chunks.len() >= 2,
            "expected >= 2 chunks when {} files exceed MAX_REVIEW_CHUNK_FILES({}), got {}",
            file_count,
            MAX_REVIEW_CHUNK_FILES,
            chunks.len()
        );
        // First chunk must not exceed the file limit
        assert!(chunks[0].len() <= MAX_REVIEW_CHUNK_FILES);

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Chunking: multiple small files fit in one chunk ──────────────

    #[test]
    fn chunk_builder_keeps_small_files_together() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-smallfiles-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let files: Vec<String> = (0..5)
            .map(|i| {
                let name = format!("s{}.rs", i);
                fs::write(workspace.join(&name), "fn s() {}\n").unwrap();
                name
            })
            .collect();

        let chunks = chunking::build_chunks(&workspace, &files).unwrap();
        assert_eq!(chunks.len(), 1, "5 tiny files should fit in one chunk");
        assert_eq!(chunks[0].len(), 5);

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Scope: Paths scope with empty vec returns empty ──────────────

    #[test]
    fn scope_paths_empty_input_returns_empty() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-empty-paths-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Paths(vec![]),
        )
        .unwrap();

        assert!(files.is_empty());
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Scope: Globs scope with empty vec returns empty ──────────────

    #[test]
    fn scope_globs_empty_input_returns_empty() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-empty-globs-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("src.rs"), "fn f() {}\n").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Globs(vec![]),
        )
        .unwrap();

        assert!(files.is_empty());
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── parse_review_response: valid JSON with findings ──────────────

    #[test]
    fn parse_review_response_normalizes_fields() {
        // Wrap in surrounding text to exercise extract_task_execution_report_json
        let response = r#"Some preamble text.

```json
{
  "outcome": "completed_no_change",
  "summary": "  Summary with spaces  ",
  "details": "  Details with spaces  ",
  "changed_files": [],
  "tests_run": ["  test1  ", "", "test2"],
  "warnings": ["warn1"],
  "blockers": [],
  "remaining_gaps": ["  gap  "],
  "design_requirements_satisfied": ["  req  "],
  "fix_tasks": [],
  "findings": [
    {
      "severity": "high",
      "confidence": "high",
      "file": "src/main.rs",
      "line": 10,
      "dimension": "correctness",
      "description": "A real bug",
      "fix_suggestion": ""
    }
  ]
}
```

Trailing text."#;
        let parsed = parse_review_response(response).unwrap();
        assert_eq!(parsed.report.summary, "Summary with spaces");
        assert_eq!(parsed.report.details, "Details with spaces");
        assert_eq!(parsed.report.tests_run, vec!["test1", "test2"]);
        assert_eq!(parsed.report.remaining_gaps, vec!["gap"]);
        assert_eq!(parsed.report.design_requirements_satisfied, vec!["req"]);
        assert_eq!(parsed.findings.len(), 1);
        assert_eq!(parsed.findings[0].severity, ReviewSeverity::High);
    }

    // ── parse_review_response: changed_files are cleaned ──────────────

    #[test]
    fn parse_review_response_rejects_nonempty_changed_files() {
        let response = r#"```json
{
  "outcome": "completed_with_changes",
  "summary": "s",
  "details": "d",
  "changed_files": ["src/main.rs"],
  "tests_run": [],
  "warnings": [],
  "blockers": [],
  "remaining_gaps": [],
  "design_requirements_satisfied": [],
  "fix_tasks": [],
  "findings": []
}
```"#;
        let result = parse_review_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("changed_files"));
    }

    // ── parse_review_response: invalid finding causes error ──────────

    #[test]
    fn parse_review_response_rejects_invalid_finding() {
        let response = r#"```json
{
  "outcome": "completed_no_change",
  "summary": "s",
  "details": "d",
  "changed_files": [],
  "tests_run": [],
  "warnings": [],
  "blockers": [],
  "remaining_gaps": [],
  "design_requirements_satisfied": [],
  "fix_tasks": [],
  "findings": [{"severity":"high","confidence":"","file":"a.rs","line":1,"dimension":"x","description":"y","fix_suggestion":""}]
}
```"#;
        let result = parse_review_response(response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("confidence"));
    }

    // ── parse_review_response: missing JSON ──────────────────────────

    #[test]
    fn parse_review_response_missing_json() {
        let result = parse_review_response("No JSON here at all");
        assert!(result.is_err());
    }

    // ── build_review_plan: tasks include profile in read_scope ────────

    #[test]
    fn build_review_plan_tasks_include_profile_in_read_scope() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-plan-scope-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("src.rs"), "fn main() {}\n").unwrap();

        let plan = build_review_plan(
            &workspace,
            &ReviewScope::Paths(vec!["src.rs".to_string()]),
            None,
            ReviewFixMode::ReportOnly,
            Vec::new(),
            Vec::new(),
            Utc::now(),
        )
        .unwrap();

        assert!(!plan.review_tasks.is_empty());
        for task in &plan.review_tasks {
            assert!(
                task.read_scope.contains(&REVIEW_PROFILE_PATH.to_string()),
                "task read_scope should contain the review profile path"
            );
        }

        let _ = fs::remove_dir_all(workspace);
    }

    // ── build_review_plan: GenerateFixes mode propagated ─────────────

    #[test]
    fn build_review_plan_generates_fix_mode_metadata() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-plan-fixmode-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("a.rs"), "fn a() {}\n").unwrap();

        let plan = build_review_plan(
            &workspace,
            &ReviewScope::Paths(vec!["a.rs".to_string()]),
            Some(&["security".to_string()]),
            ReviewFixMode::GenerateFixes,
            Vec::new(),
            vec!["stale profile".to_string()],
            Utc::now(),
        )
        .unwrap();

        let meta = plan.review_tasks[0].metadata.as_ref().unwrap();
        assert_eq!(meta.fix_mode, Some(ReviewFixMode::GenerateFixes));
        assert!(meta.profile_warnings.contains(&"stale profile".to_string()));
        assert!(meta.dimensions.contains(&"security".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Scope: mixed directory and file paths ────────────────────────

    #[test]
    fn scope_paths_mixed_directory_and_file() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-mixed-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(workspace.join("root.rs"), "fn root() {}\n").unwrap();

        let (mut files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Paths(vec!["src".to_string(), "root.rs".to_string()]),
        )
        .unwrap();
        files.sort();

        assert_eq!(
            files,
            vec!["root.rs".to_string(), "src/lib.rs".to_string()]
        );
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Chunking: build_chunks with zero files ──────────────────────

    #[test]
    fn chunk_builder_empty_files_returns_empty() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-nofiles-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let chunks = chunking::build_chunks(&workspace, &[]).unwrap();
        assert!(chunks.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── ReviewSeverity serde round-trip ──────────────────────────────

    #[test]
    fn review_severity_serde_round_trip() {
        for (severity, expected_json) in [
            (ReviewSeverity::High, "\"high\""),
            (ReviewSeverity::Medium, "\"medium\""),
            (ReviewSeverity::Low, "\"low\""),
        ] {
            let json = serde_json::to_string(&severity).unwrap();
            assert_eq!(json, expected_json);
            let decoded: ReviewSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, severity);
        }
    }

    // ── AutomatedToolCheck serde round-trip ──────────────────────────

    #[test]
    fn automated_tool_check_serde_round_trip() {
        let check = AutomatedToolCheck {
            command: "cargo test".to_string(),
            status: "passed".to_string(),
            summary: "ok".to_string(),
        };
        let json = serde_json::to_string(&check).unwrap();
        let decoded: AutomatedToolCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, check);
    }

    // ── AutomatedToolCheck default ──────────────────────────────────

    #[test]
    fn automated_tool_check_default_is_empty() {
        let check = AutomatedToolCheck::default();
        assert!(check.command.is_empty());
        assert!(check.status.is_empty());
        assert!(check.summary.is_empty());
    }

    #[test]
    fn validate_review_response_passes_with_valid_findings() {
        let response = ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::Medium,
                confidence: "high".to_string(),
                file: "src/a.rs".to_string(),
                line: 10,
                dimension: "code_quality".to_string(),
                description: "Valid finding".to_string(),
                fix_suggestion: String::new(),
            }],
        };
        assert!(validate_review_response(&response).is_ok());
    }
