    #[test]
    fn review_plan_and_metadata_round_trip() {
        let plan = CodeReviewPlanArtifact {
            scope_description: "Current workspace".to_string(),
            output_path: "docs/code-review-2026-04-04.md".to_string(),
            automated_checks: vec![AutomatedToolCheck {
                command: "cargo check".to_string(),
                status: "passed".to_string(),
                summary: "ok".to_string(),
            }],
            profile_warnings: vec!["stale".to_string()],
            review_tasks: vec![ReviewTaskPlan {
                title: "Review chunk 1".to_string(),
                prompt: "Prompt".to_string(),
                read_scope: vec!["src/lib.rs".to_string()],
                metadata: Some(ReviewTaskMetadata {
                    scope_description: "Current workspace".to_string(),
                    dimensions: vec!["architecture".to_string()],
                    chunk: vec![ReviewChunkTarget {
                        path: "src/lib.rs".to_string(),
                        start_line: 1,
                        end_line: 25,
                    }],
                    automated_checks: Vec::new(),
                    profile_path: REVIEW_PROFILE_PATH.to_string(),
                    profile_warnings: Vec::new(),
                    fix_mode: Some(ReviewFixMode::ReportOnly),
                    output_path: None,
                }),
            }],
        };

        let json = serde_json::to_string(&plan).expect("plan should serialize");
        let parsed: CodeReviewPlanArtifact =
            serde_json::from_str(&json).expect("plan should deserialize");
        assert_eq!(parsed, plan);
    }

    #[test]
    fn rendered_review_document_orders_by_severity_then_location() {
        let metadata = ReviewTaskMetadata {
            scope_description: "Current workspace".to_string(),
            dimensions: vec!["architecture".to_string()],
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: Some(ReviewFixMode::ReportOnly),
            output_path: None,
        };
        let doc = render_review_document(
            "rust-chatbot",
            NaiveDate::from_ymd_opt(2026, 4, 4).unwrap(),
            "Current workspace",
            &metadata,
            &[ReviewArtifactResponse {
                report: TaskExecutionReport::default(),
                findings: vec![
                    ReviewFinding {
                        severity: ReviewSeverity::Low,
                        confidence: "medium".to_string(),
                        file: "src/z.rs".to_string(),
                        line: 40,
                        dimension: "maintainability".to_string(),
                        description: "Low issue".to_string(),
                        fix_suggestion: String::new(),
                    },
                    ReviewFinding {
                        severity: ReviewSeverity::High,
                        confidence: "high".to_string(),
                        file: "src/a.rs".to_string(),
                        line: 12,
                        dimension: "architecture".to_string(),
                        description: "High issue".to_string(),
                        fix_suggestion: String::new(),
                    },
                ],
            }],
        );

        let high_index = doc.find("1. High: High issue").unwrap();
        let low_index = doc.find("2. Low: Low issue").unwrap();
        assert!(high_index < low_index);
    }

    #[test]
    fn glob_scope_matches_reviewable_files() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-globs-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src/nested")).unwrap();
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(workspace.join("src/nested/mod.rs"), "pub fn nested() {}\n").unwrap();
        fs::write(workspace.join("docs/readme.md"), "# readme\n").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Globs(vec!["src/**/*.rs".to_string(), "docs/*.md".to_string()]),
        )
        .unwrap();

        assert_eq!(
            files,
            vec![
                "docs/readme.md".to_string(),
                "src/lib.rs".to_string(),
                "src/nested/mod.rs".to_string(),
            ]
        );
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn worktree_scope_skips_runtime_review_dirs() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-skip-dirs-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join(".claude-desktop/telemetry")).unwrap();
        fs::create_dir_all(workspace.join(".rag_model_cache/models")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(workspace.join(".mcp.json"), "{\"tools\":[]}\n").unwrap();
        fs::write(
            workspace.join(".claude-desktop/telemetry/event.json"),
            "{\"ignored\":true}\n",
        )
        .unwrap();
        fs::write(
            workspace.join(".rag_model_cache/models/model.json"),
            "{\"ignored\":true}\n",
        )
        .unwrap();

        let (mut files, warnings) =
            scope::resolve_scope_files(&workspace, &ReviewScope::Worktree).unwrap();
        files.sort();

        assert_eq!(
            files,
            vec![".mcp.json".to_string(), "src/lib.rs".to_string()]
        );
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn worktree_scope_respects_gitignore() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-gitignore-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join(".generated")).unwrap();
        let git_init = Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .expect("git init should succeed");
        assert!(git_init.status.success());
        fs::write(workspace.join(".gitignore"), ".generated/\nignored.json\n").unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(
            workspace.join(".generated/cache.json"),
            "{\"ignored\":true}\n",
        )
        .unwrap();
        fs::write(workspace.join("ignored.json"), "{\"ignored\":true}\n").unwrap();

        let (mut files, warnings) =
            scope::resolve_scope_files(&workspace, &ReviewScope::Worktree).unwrap();
        files.sort();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn worktree_scope_respects_nested_gitignores() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-nested-gitignore-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join("fixtures/generated")).unwrap();
        let git_init = Command::new("git")
            .args(["init"])
            .current_dir(&workspace)
            .output()
            .expect("git init should succeed");
        assert!(git_init.status.success());
        fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();
        fs::write(
            workspace.join("fixtures/.gitignore"),
            "generated/\nignored.md\n",
        )
        .unwrap();
        fs::write(
            workspace.join("fixtures/generated/cache.json"),
            "{\"ignored\":true}\n",
        )
        .unwrap();
        fs::write(workspace.join("fixtures/ignored.md"), "# ignored\n").unwrap();
        fs::write(workspace.join("fixtures/kept.md"), "# kept\n").unwrap();

        let (mut files, warnings) =
            scope::resolve_scope_files(&workspace, &ReviewScope::Worktree).unwrap();
        files.sort();

        assert_eq!(
            files,
            vec!["fixtures/kept.md".to_string(), "src/lib.rs".to_string()]
        );
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn chunk_builder_limits_files_per_chunk() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-chunk-files-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();

        let mut files = Vec::new();
        for index in 0..(MAX_REVIEW_CHUNK_FILES + 1) {
            let rel = format!("src/file-{index}.rs");
            fs::write(workspace.join(&rel), "pub fn f() {}\n").unwrap();
            files.push(rel);
        }

        let chunks = chunking::build_chunks(&workspace, &files).unwrap();

        assert_eq!(chunks.len(), 2);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.len() <= MAX_REVIEW_CHUNK_FILES));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── glob_matches_path ────────────────────────────────────────────

    #[test]
    fn glob_matches_exact_path() {
        assert!(scope::glob_matches_path_test("src/lib.rs", "src/lib.rs"));
    }

    #[test]
    fn glob_star_matches_single_component() {
        assert!(scope::glob_matches_path_test("src/*.rs", "src/lib.rs"));
        assert!(!scope::glob_matches_path_test("src/*.rs", "src/sub/lib.rs"));
    }

    #[test]
    fn glob_double_star_matches_nested() {
        assert!(scope::glob_matches_path_test("**/*.rs", "src/lib.rs"));
        assert!(scope::glob_matches_path_test("**/*.rs", "src/deep/nested/lib.rs"));
    }

    #[test]
    fn glob_double_star_at_start_matches_root() {
        assert!(scope::glob_matches_path_test("**/*.rs", "lib.rs"));
    }

    #[test]
    fn glob_no_match() {
        assert!(!scope::glob_matches_path_test("src/*.rs", "tests/main.rs"));
    }

    #[test]
    fn glob_question_mark_matches_single_char() {
        assert!(scope::glob_matches_path_test("src/?.rs", "src/a.rs"));
        assert!(!scope::glob_matches_path_test("src/?.rs", "src/ab.rs"));
    }

    // ── should_skip_dir ──────────────────────────────────────────────

    #[test]
    fn should_skip_known_dirs() {
        for name in &[".git", "target", "node_modules", "dist", ".claude-desktop", ".rag_model_cache"] {
            assert!(
                scope::should_skip_dir_test(Path::new(name)),
                "expected to skip {}",
                name
            );
        }
    }

    #[test]
    fn should_not_skip_src() {
        assert!(!scope::should_skip_dir_test(Path::new("src")));
    }

    // ── is_reviewable_path ───────────────────────────────────────────

    #[test]
    fn reviewable_extensions() {
        let dir = std::env::temp_dir().join(format!(
            "rust-chatbot-review-ext-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();

        for ext in &["rs", "toml", "md", "json", "yaml", "yml", "sh", "ts", "tsx", "js", "jsx"] {
            let file = dir.join(format!("file.{}", ext));
            fs::write(&file, "content").unwrap();
            assert!(scope::is_reviewable_path_test(&file), "expected {} to be reviewable", ext);
        }

        let non_reviewable = dir.join("file.exe");
        fs::write(&non_reviewable, "content").unwrap();
        assert!(!scope::is_reviewable_path_test(&non_reviewable));

        let _ = fs::remove_dir_all(dir);
    }

    // ── split_large_file ─────────────────────────────────────────────

    #[test]
    fn split_large_file_at_impl_boundaries() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-split-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let mut content = String::new();
        content.push_str("impl Foo {\n");
        for i in 0..1000 {
            content.push_str(&format!("    fn method_{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");
        content.push_str("impl Bar {\n");
        for i in 0..1500 {
            content.push_str(&format!("    fn method_{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");

        let file = "big.rs";
        fs::write(workspace.join(file), &content).unwrap();

        let chunks = chunking::build_chunks(&workspace, &[file.to_string()]).unwrap();
        assert!(chunks.len() >= 2, "expected at least 2 chunks, got {}", chunks.len());

        let targets: Vec<_> = chunks.into_iter().flatten().collect();
        for window in targets.windows(2) {
            assert_eq!(
                window[0].end_line + 1,
                window[1].start_line,
                "chunks should be contiguous"
            );
        }

        let _ = fs::remove_dir_all(workspace);
    }

    // ── build_chunk_title ────────────────────────────────────────────

    #[test]
    fn chunk_title_single_file() {
        let chunk = vec![ReviewChunkTarget {
            path: "src/main.rs".to_string(),
            start_line: 1,
            end_line: 100,
        }];
        let title = chunking::build_chunk_title(1, &chunk);
        assert!(title.contains("src/main.rs"));
        assert!(title.contains("1-100"));
    }

    #[test]
    fn chunk_title_multiple_files() {
        let chunk = vec![
            ReviewChunkTarget {
                path: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
            },
            ReviewChunkTarget {
                path: "b.rs".to_string(),
                start_line: 1,
                end_line: 20,
            },
        ];
        let title = chunking::build_chunk_title(2, &chunk);
        assert!(title.contains("2 files"));
    }

    // ── dimension_guidance ───────────────────────────────────────────

    #[test]
    fn dimension_guidance_known_dimensions() {
        let known = [
            "architecture",
            "code_quality",
            "correctness",
            "comment_accuracy",
            "test_coverage",
            "error_handling",
            "type_design",
            "simplification",
            "maintainability",
            "security",
            "performance",
            "concurrency",
        ];
        for dim in &known {
            let guidance = chunking::dimension_guidance(dim);
            assert!(
                !guidance.contains("Apply this dimension directly"),
                "expected specific guidance for {}",
                dim
            );
        }
    }

    #[test]
    fn dimension_guidance_unknown_returns_generic() {
        let guidance = chunking::dimension_guidance("unknown_dim");
        assert!(guidance.contains("Apply this dimension directly"));
    }

    // ── ReviewSeverity ───────────────────────────────────────────────

    #[test]
    fn review_severity_display_names() {
        assert_eq!(ReviewSeverity::High.display_name(), "High");
        assert_eq!(ReviewSeverity::Medium.display_name(), "Medium");
        assert_eq!(ReviewSeverity::Low.display_name(), "Low");
    }

    #[test]
    fn review_severity_ordering() {
        assert!(ReviewSeverity::High < ReviewSeverity::Medium);
        assert!(ReviewSeverity::Medium < ReviewSeverity::Low);
    }

    // ── describe_scope edge cases ───────────────────────────────────

    #[test]
    fn describe_scope_worktree() {
        assert_eq!(describe_scope(&ReviewScope::Worktree), "Current workspace");
    }

    #[test]
    fn describe_scope_diff() {
        let scope = ReviewScope::Diff {
            base: "HEAD~3".to_string(),
        };
        assert_eq!(describe_scope(&scope), "Changes since `HEAD~3`");
    }

    #[test]
    fn describe_scope_empty_paths() {
        let scope = ReviewScope::Paths(vec![]);
        assert_eq!(describe_scope(&scope), "Specific paths");
    }

    #[test]
    fn describe_scope_empty_globs() {
        let scope = ReviewScope::Globs(vec![]);
        assert_eq!(describe_scope(&scope), "Glob patterns");
    }

    #[test]
    fn describe_scope_multiple_paths() {
        let scope = ReviewScope::Paths(vec!["src/a.rs".into(), "src/b.rs".into()]);
        let desc = describe_scope(&scope);
        assert!(desc.contains("src/a.rs"));
        assert!(desc.contains("src/b.rs"));
    }

    // ── review_profile_warnings branches ────────────────────────────

    #[test]
    fn review_profile_warnings_missing_profile() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-profile-missing-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        let now = Utc::now();
        let warnings = review_profile_warnings(&workspace, now);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("missing"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn review_profile_warnings_fresh_profile() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-profile-fresh-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join(REVIEW_PROFILE_PATH), "# Profile\n").unwrap();
        let now = Utc::now();
        let warnings = review_profile_warnings(&workspace, now);
        assert!(warnings.is_empty(), "fresh profile should have no warnings");
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn review_profile_warnings_stale_when_future_now() {
        // Simulate stale profile by passing a "now" far in the future
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-profile-stale-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("docs")).unwrap();
        fs::write(workspace.join(REVIEW_PROFILE_PATH), "# Profile\n").unwrap();
        // Use a date 60 days in the future to make the profile "stale"
        let future_now = Utc::now() + Duration::days(60);
        let warnings = review_profile_warnings(&workspace, future_now);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("older than"));
        let _ = fs::remove_dir_all(workspace);
    }

    // ── validate_finding error branches ─────────────────────────────

    #[test]
    fn validate_finding_missing_confidence() {
        let result = validate_review_response(&ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                confidence: "  ".to_string(),
                file: "src/a.rs".to_string(),
                line: 1,
                dimension: "architecture".to_string(),
                description: "desc".to_string(),
                fix_suggestion: String::new(),
            }],
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("confidence"));
    }

    #[test]
    fn validate_finding_missing_file() {
        let result = validate_review_response(&ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                confidence: "high".to_string(),
                file: "".to_string(),
                line: 1,
                dimension: "architecture".to_string(),
                description: "desc".to_string(),
                fix_suggestion: String::new(),
            }],
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("file"));
    }

    #[test]
    fn validate_finding_zero_line() {
        let result = validate_review_response(&ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                confidence: "high".to_string(),
                file: "src/a.rs".to_string(),
                line: 0,
                dimension: "architecture".to_string(),
                description: "desc".to_string(),
                fix_suggestion: String::new(),
            }],
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("line"));
    }

    #[test]
    fn validate_finding_missing_dimension() {
        let result = validate_review_response(&ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                confidence: "high".to_string(),
                file: "src/a.rs".to_string(),
                line: 1,
                dimension: "  ".to_string(),
                description: "desc".to_string(),
                fix_suggestion: String::new(),
            }],
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dimension"));
    }

    #[test]
    fn validate_finding_missing_description() {
        let result = validate_review_response(&ReviewArtifactResponse {
            report: TaskExecutionReport::default(),
            findings: vec![ReviewFinding {
                severity: ReviewSeverity::High,
                confidence: "high".to_string(),
                file: "src/a.rs".to_string(),
                line: 1,
                dimension: "architecture".to_string(),
                description: "".to_string(),
                fix_suggestion: String::new(),
            }],
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("description"));
    }

    // ── validate_fix_mode ───────────────────────────────────────────

    #[test]
    fn validate_fix_mode_report_only_rejects_fix_tasks() {
        // Build a response with fix_tasks via JSON deserialization since fields are private
        let json = r#"{"outcome":"completed_no_change","summary":"s","details":"d","changed_files":[],"tests_run":[],"warnings":[],"blockers":[],"remaining_gaps":[],"design_requirements_satisfied":[],"fix_tasks":[{"title":"fix it","prompt":"p","kind":"write"}],"findings":[]}"#;
        let response: ReviewArtifactResponse = serde_json::from_str(json).unwrap();
        let metadata = ReviewTaskMetadata {
            scope_description: String::new(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: String::new(),
            profile_warnings: Vec::new(),
            fix_mode: Some(ReviewFixMode::ReportOnly),
            output_path: None,
        };
        let result = validate_fix_mode(&response, &metadata);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("report-only"));
    }

    #[test]
    fn validate_fix_mode_generate_fixes_allows_fix_tasks() {
        let json = r#"{"outcome":"completed_no_change","summary":"s","details":"d","changed_files":[],"tests_run":[],"warnings":[],"blockers":[],"remaining_gaps":[],"design_requirements_satisfied":[],"fix_tasks":[{"title":"fix it","prompt":"p","kind":"write"}],"findings":[]}"#;
        let response: ReviewArtifactResponse = serde_json::from_str(json).unwrap();
        let metadata = ReviewTaskMetadata {
            scope_description: String::new(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: String::new(),
            profile_warnings: Vec::new(),
            fix_mode: Some(ReviewFixMode::GenerateFixes),
            output_path: None,
        };
        assert!(validate_fix_mode(&response, &metadata).is_ok());
    }

    // ── render_review_document edge cases ────────────────────────────

    #[test]
    fn render_review_document_empty_findings() {
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
        assert!(doc.contains("No review findings were recorded"));
        assert!(doc.contains("None recorded"));
    }

    #[test]
    fn render_review_document_deduplicates_findings() {
        let finding = ReviewFinding {
            severity: ReviewSeverity::High,
            confidence: "high".to_string(),
            file: "src/a.rs".to_string(),
            line: 10,
            dimension: "architecture".to_string(),
            description: "Duplicate issue".to_string(),
            fix_suggestion: String::new(),
        };
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
            &[
                ReviewArtifactResponse {
                    report: TaskExecutionReport::default(),
                    findings: vec![finding.clone()],
                },
                ReviewArtifactResponse {
                    report: TaskExecutionReport::default(),
                    findings: vec![finding],
                },
            ],
        );
        // Should only appear once despite being in two artifacts
        let count = doc.matches("Duplicate issue").count();
        assert_eq!(count, 1, "duplicates should be removed");
    }

    #[test]
    fn render_review_document_includes_fix_suggestion() {
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
                    severity: ReviewSeverity::Medium,
                    confidence: "high".to_string(),
                    file: "src/a.rs".to_string(),
                    line: 5,
                    dimension: "code_quality".to_string(),
                    description: "Bug here".to_string(),
                    fix_suggestion: "Use Option instead".to_string(),
                }],
            }],
        );
        assert!(doc.contains("Suggested fix: Use Option instead"));
    }

    // ── scope resolution: Paths with missing files ──────────────────

    #[test]
    fn scope_paths_warns_on_missing_path() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-missing-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Paths(vec![
                "src/lib.rs".to_string(),
                "src/nonexistent.rs".to_string(),
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("does not exist"));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── scope resolution: Globs with no matches ─────────────────────

