    #[test]
    fn scope_globs_warns_on_no_match() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-noglob-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();

        let (_files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Globs(vec!["docs/**/*.py".to_string()]),
        )
        .unwrap();

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("matched no reviewable files"));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── chunking: fixed-width split (no impl boundaries) ────────────

    #[test]
    fn chunk_splits_large_file_without_impl_boundaries() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-fixedwidth-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        // Generate a large file with no `impl` blocks
        let mut content = String::new();
        for i in 0..3000 {
            content.push_str(&format!("fn func_{}() {{}}\n", i));
        }
        fs::write(workspace.join("large.rs"), &content).unwrap();

        let chunks = chunking::build_chunks(&workspace, &["large.rs".to_string()]).unwrap();
        assert!(chunks.len() >= 2, "expected split, got {} chunks", chunks.len());

        // Every chunk target should respect MAX_REVIEW_CHUNK_LINES
        for chunk in &chunks {
            for target in chunk {
                let lines = target.end_line - target.start_line + 1;
                assert!(
                    lines <= MAX_REVIEW_CHUNK_LINES,
                    "chunk has {} lines, max is {}",
                    lines,
                    MAX_REVIEW_CHUNK_LINES
                );
            }
        }

        let _ = fs::remove_dir_all(workspace);
    }

    // ── chunking: line-count limit triggers new chunk ────────────────

    #[test]
    fn chunk_builder_splits_on_line_count() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-linelimit-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        // Create two files that together exceed MAX_REVIEW_CHUNK_LINES
        let mut content = String::new();
        for i in 0..1200 {
            content.push_str(&format!("fn f{}() {{}}\n", i));
        }
        fs::write(workspace.join("a.rs"), &content).unwrap();
        fs::write(workspace.join("b.rs"), &content).unwrap();

        let chunks = chunking::build_chunks(
            &workspace,
            &["a.rs".to_string(), "b.rs".to_string()],
        )
        .unwrap();
        assert!(
            chunks.len() >= 2,
            "expected at least 2 chunks from line overflow, got {}",
            chunks.len()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    // ── dimensions_for_chunk auto-detection ──────────────────────────

    #[test]
    fn dimensions_detects_error_handling_and_type_design() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(
            workspace.join("types.rs"),
            "struct Foo {}\nenum Bar {}\nfn err() -> Result<(), anyhow::Error> { bail!(\"oops\") }\n/// doc comment\n",
        )
        .unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "types.rs".to_string(),
            start_line: 1,
            end_line: 4,
        }];
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, None).unwrap();
        assert!(dims.contains(&"error_handling".to_string()));
        assert!(dims.contains(&"type_design".to_string()));
        assert!(dims.contains(&"comment_accuracy".to_string()));
        assert!(dims.contains(&"architecture".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn dimensions_selected_override_adds_to_auto() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-sel-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("simple.rs"), "fn main() {}\n").unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "simple.rs".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        let selected = vec!["security".to_string()];
        let dims =
            chunking::dimensions_for_chunk(&workspace, &chunk, Some(&selected)).unwrap();
        assert!(dims.contains(&"security".to_string()));
        assert!(dims.contains(&"architecture".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── normalize_string_list ────────────────────────────────────────

    #[test]
    fn normalize_string_list_filters_empty_and_trims() {
        let result = normalize_string_list(vec![
            "  hello  ".to_string(),
            "".to_string(),
            "  ".to_string(),
            "world".to_string(),
        ]);
        assert_eq!(result, vec!["hello", "world"]);
    }

    // ── validate_review_response rejects changed_files ───────────────

    #[test]
    fn validate_review_response_rejects_changed_files() {
        let response = ReviewArtifactResponse {
            report: TaskExecutionReport {
                changed_files: vec!["src/a.rs".to_string()],
                ..Default::default()
            },
            findings: Vec::new(),
        };
        let result = validate_review_response(&response);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("changed_files"));
    }

    // ── build_chunk_prompt ──────────────────────────────────────────

    #[test]
    fn build_chunk_prompt_includes_scope_and_dimensions() {
        let metadata = ReviewTaskMetadata {
            scope_description: "Workspace scope".to_string(),
            dimensions: vec!["architecture".to_string(), "security".to_string()],
            chunk: vec![ReviewChunkTarget {
                path: "src/main.rs".to_string(),
                start_line: 1,
                end_line: 50,
            }],
            automated_checks: vec![AutomatedToolCheck {
                command: "cargo check".to_string(),
                status: "passed".to_string(),
                summary: "ok".to_string(),
            }],
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: vec!["stale profile".to_string()],
            fix_mode: Some(ReviewFixMode::GenerateFixes),
            output_path: None,
        };
        let prompt = chunking::build_chunk_prompt(&metadata);
        assert!(prompt.contains("Workspace scope"));
        assert!(prompt.contains("src/main.rs:1-50"));
        assert!(prompt.contains("`architecture`"));
        assert!(prompt.contains("`security`"));
        assert!(prompt.contains("cargo check: passed"));
        assert!(prompt.contains("stale profile"));
        assert!(prompt.contains("emit small `fix_tasks`"));
    }

    #[test]
    fn build_chunk_prompt_report_only_mode() {
        let metadata = ReviewTaskMetadata {
            scope_description: "test".to_string(),
            dimensions: vec!["code_quality".to_string()],
            chunk: vec![ReviewChunkTarget {
                path: "a.rs".to_string(),
                start_line: 1,
                end_line: 10,
            }],
            automated_checks: Vec::new(),
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: Vec::new(),
            fix_mode: Some(ReviewFixMode::ReportOnly),
            output_path: None,
        };
        let prompt = chunking::build_chunk_prompt(&metadata);
        assert!(prompt.contains("report-only"));
        assert!(prompt.contains("Keep `fix_tasks` empty"));
    }

    // ── glob matching edge cases ────────────────────────────────────

    #[test]
    fn glob_double_star_mid_path() {
        assert!(scope::glob_matches_path_test("src/**/test.rs", "src/a/b/test.rs"));
        assert!(scope::glob_matches_path_test("src/**/test.rs", "src/test.rs"));
    }

    #[test]
    fn glob_star_matches_empty_component() {
        // star at component level should match any filename
        assert!(scope::glob_matches_path_test("src/*", "src/anything"));
    }

    #[test]
    fn glob_backslash_normalized() {
        // scope.rs normalize_glob_pattern replaces backslashes
        let normalized = "src\\nested\\file.rs".replace('\\', "/");
        assert!(scope::glob_matches_path_test(&normalized, "src/nested/file.rs"));
    }

    // ── code_review_doc_path ─────────────────────────────────────────

    #[test]
    fn code_review_doc_path_formats_date() {
        let now = Utc.with_ymd_and_hms(2026, 4, 5, 10, 0, 0).unwrap();
        assert_eq!(code_review_doc_path(now), "docs/code-review-2026-04-05.md");
    }

    // ── build_review_plan edge cases ─────────────────────────────────

    #[test]
    fn build_review_plan_empty_scope_bails() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-plan-empty-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        // Empty glob pattern that matches nothing
        let result = build_review_plan(
            &workspace,
            &ReviewScope::Globs(vec!["nonexistent/**/*.xyz".to_string()]),
            None,
            ReviewFixMode::ReportOnly,
            Vec::new(),
            Vec::new(),
            Utc::now(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no reviewable files"));
        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn build_review_plan_produces_tasks_with_read_scope() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-plan-scope-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "pub fn lib() {}\n").unwrap();

        let plan = build_review_plan(
            &workspace,
            &ReviewScope::Paths(vec!["src/lib.rs".to_string()]),
            None,
            ReviewFixMode::GenerateFixes,
            Vec::new(),
            vec!["stale profile".to_string()],
            Utc::now(),
        )
        .unwrap();

        assert!(!plan.review_tasks.is_empty());
        let task = &plan.review_tasks[0];
        assert!(task.read_scope.contains(&"src/lib.rs".to_string()));
        // Profile path should also be in read_scope
        assert!(task.read_scope.contains(&REVIEW_PROFILE_PATH.to_string()));
        // Profile warnings should be in metadata
        let metadata = task.metadata.as_ref().unwrap();
        assert!(metadata.profile_warnings.contains(&"stale profile".to_string()));
        let _ = fs::remove_dir_all(workspace);
    }

    // ── chunking: oversized impl block triggers fixed-width split ─────

    #[test]
    fn chunk_splits_oversized_impl_block_with_fixed_width() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-bigimpl-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        // A single impl block exceeding MAX_REVIEW_CHUNK_LINES
        let mut content = String::new();
        content.push_str("impl Foo {\n");
        for i in 0..(MAX_REVIEW_CHUNK_LINES + 500) {
            content.push_str(&format!("    fn m_{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");
        content.push_str("impl Bar {\n");
        content.push_str("    fn bar(&self) {}\n");
        content.push_str("}\n");

        fs::write(workspace.join("big_impl.rs"), &content).unwrap();
        let chunks = chunking::build_chunks(&workspace, &["big_impl.rs".to_string()]).unwrap();
        assert!(
            chunks.len() >= 2,
            "oversized impl block should be split into multiple chunks, got {}",
            chunks.len()
        );

        // All targets should respect MAX_REVIEW_CHUNK_LINES
        for chunk in &chunks {
            for target in chunk {
                let lines = target.end_line - target.start_line + 1;
                assert!(
                    lines <= MAX_REVIEW_CHUNK_LINES,
                    "target has {} lines, max is {}",
                    lines,
                    MAX_REVIEW_CHUNK_LINES
                );
            }
        }

        let _ = fs::remove_dir_all(workspace);
    }

    // ── dimensions_for_chunk: test path and .md path detection ────────

    #[test]
    fn dimensions_detects_test_coverage_for_test_paths() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("tests")).unwrap();
        fs::write(workspace.join("tests/integration.rs"), "fn test_it() {}\n").unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "tests/integration.rs".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, None).unwrap();
        assert!(dims.contains(&"test_coverage".to_string()));
        assert!(dims.contains(&"architecture".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn dimensions_detects_comment_accuracy_for_md_files() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-md-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("readme.md"), "# Hello\n").unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "readme.md".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, None).unwrap();
        assert!(dims.contains(&"comment_accuracy".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn dimensions_empty_selected_adds_code_quality() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-dims-empty-sel-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("plain.rs"), "fn plain() {}\n").unwrap();

        let chunk = vec![ReviewChunkTarget {
            path: "plain.rs".to_string(),
            start_line: 1,
            end_line: 1,
        }];
        // Pass Some(&[]) — empty selected dimensions
        let dims = chunking::dimensions_for_chunk(&workspace, &chunk, Some(&[])).unwrap();
        assert!(dims.contains(&"code_quality".to_string()));
        assert!(dims.contains(&"architecture".to_string()));

        let _ = fs::remove_dir_all(workspace);
    }

    // ── render_review_document: testing_gaps and fix_status ───────────

    #[test]
    fn render_review_document_includes_testing_gaps_and_fix_status() {
        let metadata = ReviewTaskMetadata {
            scope_description: "workspace".to_string(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: vec![AutomatedToolCheck {
                command: "cargo test".to_string(),
                status: "passed".to_string(),
                summary: "ok".to_string(),
            }],
            profile_path: REVIEW_PROFILE_PATH.to_string(),
            profile_warnings: vec!["profile warning".to_string()],
            fix_mode: None,
            output_path: None,
        };
        let json = r#"{"outcome":"completed_no_change","summary":"s","details":"d","changed_files":[],"tests_run":[],"warnings":["runtime warning"],"blockers":[],"remaining_gaps":["missing unit tests for edge case"],"design_requirements_satisfied":[],"fix_tasks":[{"title":"Fix edge case","prompt":"p","kind":"write"}],"findings":[{"severity":"medium","confidence":"high","file":"src/a.rs","line":5,"dimension":"code_quality","description":"Issue found","fix_suggestion":""}]}"#;
        let artifact: ReviewArtifactResponse = serde_json::from_str(json).unwrap();

        let doc = render_review_document(
            "test-proj",
            NaiveDate::from_ymd_opt(2026, 4, 5).unwrap(),
            "workspace",
            &metadata,
            &[artifact],
        );

        assert!(doc.contains("missing unit tests for edge case"));
        assert!(doc.contains("Fix edge case"));
        assert!(doc.contains("runtime warning"));
        assert!(doc.contains("profile warning"));
        assert!(doc.contains("cargo test: passed"));
    }

    // ── scope resolution: Diff scope with non-git directory ───────────

    #[test]
    fn scope_diff_non_git_directory_warns() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-diff-nongit-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Diff {
                base: "HEAD~1".to_string(),
            },
        )
        .unwrap();

        assert!(files.is_empty());
        assert!(!warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── scope resolution: Paths with directory expansion ──────────────

    #[test]
    fn scope_paths_expands_directories() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-dir-expand-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src/sub")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(workspace.join("src/sub/mod.rs"), "fn sub() {}\n").unwrap();

        let (mut files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Paths(vec!["src".to_string()]),
        )
        .unwrap();
        files.sort();

        assert_eq!(
            files,
            vec!["src/lib.rs".to_string(), "src/sub/mod.rs".to_string()]
        );
        assert!(warnings.is_empty());

        let _ = fs::remove_dir_all(workspace);
    }

    // ── glob matching: trailing ** ────────────────────────────────────

    #[test]
    fn glob_trailing_double_star() {
        assert!(scope::glob_matches_path_test("src/**", "src/a/b/c.rs"));
        assert!(scope::glob_matches_path_test("src/**", "src/a.rs"));
    }

    #[test]
    fn glob_empty_pattern_no_match() {
        assert!(!scope::glob_matches_path_test("", "src/lib.rs"));
    }

    // ── validate_fix_mode with None fix_mode defaults to ReportOnly ──

    #[test]
    fn validate_fix_mode_none_defaults_to_report_only() {
        let json = r#"{"outcome":"completed_no_change","summary":"s","details":"d","changed_files":[],"tests_run":[],"warnings":[],"blockers":[],"remaining_gaps":[],"design_requirements_satisfied":[],"fix_tasks":[{"title":"fix","prompt":"p","kind":"write"}],"findings":[]}"#;
        let response: ReviewArtifactResponse = serde_json::from_str(json).unwrap();
        let metadata = ReviewTaskMetadata {
            scope_description: String::new(),
            dimensions: Vec::new(),
            chunk: Vec::new(),
            automated_checks: Vec::new(),
            profile_path: String::new(),
            profile_warnings: Vec::new(),
            fix_mode: None, // defaults to ReportOnly
            output_path: None,
        };
        let result = validate_fix_mode(&response, &metadata);
        assert!(result.is_err());
    }

    // ── Chunking edge cases: single-line, empty, exact boundary ──────

    #[test]
    fn chunk_single_line_file() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-singleline-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("one.rs"), "fn one() {}\n").unwrap();

        let chunks = chunking::build_chunks(&workspace, &["one.rs".to_string()]).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1);
        assert_eq!(chunks[0][0].start_line, 1);
        assert_eq!(chunks[0][0].end_line, 1);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn chunk_empty_file_gets_single_line_segment() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-empty-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("empty.rs"), "").unwrap();

        let chunks = chunking::build_chunks(&workspace, &["empty.rs".to_string()]).unwrap();
        assert_eq!(chunks.len(), 1);
        // max(1) ensures at least line 1
        assert_eq!(chunks[0][0].start_line, 1);
        assert_eq!(chunks[0][0].end_line, 1);

        let _ = fs::remove_dir_all(workspace);
    }

    #[test]
    fn chunk_file_at_exact_limit_stays_single_chunk() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-exact-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();
        let mut content = String::new();
        for i in 0..MAX_REVIEW_CHUNK_LINES {
            content.push_str(&format!("fn f{}() {{}}\n", i));
        }
        fs::write(workspace.join("exact.rs"), &content).unwrap();

        let chunks = chunking::build_chunks(&workspace, &["exact.rs".to_string()]).unwrap();
        assert_eq!(chunks.len(), 1, "file at exact limit should not split");

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Chunking: impl< generic detection ────────────────────────────

    #[test]
    fn split_large_file_detects_generic_impl_boundaries() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-impl-generic-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let mut content = String::new();
        content.push_str("impl<T> Foo<T> {\n");
        for i in 0..1200 {
            content.push_str(&format!("    fn m{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");
        content.push_str("impl<U: Clone> Bar<U> {\n");
        for i in 0..1200 {
            content.push_str(&format!("    fn n{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");

        fs::write(workspace.join("generics.rs"), &content).unwrap();
        let chunks = chunking::build_chunks(&workspace, &["generics.rs".to_string()]).unwrap();
        assert!(
            chunks.len() >= 2,
            "generic impl blocks should split, got {} chunks",
            chunks.len()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Chunking: accumulation flush before oversized impl ───────────

    #[test]
    fn chunk_flushes_accumulated_before_oversized_impl() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-review-flush-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&workspace).unwrap();

        let mut content = String::new();
        // Small impl first
        content.push_str("impl Small {\n");
        for i in 0..100 {
            content.push_str(&format!("    fn s{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");
        // Oversized impl
        content.push_str("impl Huge {\n");
        for i in 0..(MAX_REVIEW_CHUNK_LINES + 200) {
            content.push_str(&format!("    fn h{}(&self) {{}}\n", i));
        }
        content.push_str("}\n");

        fs::write(workspace.join("flush.rs"), &content).unwrap();
        let chunks = chunking::build_chunks(&workspace, &["flush.rs".to_string()]).unwrap();
        // At least 3: small block, 2+ from oversized split
        assert!(
            chunks.len() >= 3,
            "expected >= 3 chunks from flush + split, got {}",
            chunks.len()
        );

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Scope resolution: non-reviewable file in Paths scope ─────────

    #[test]
    fn scope_paths_skips_non_reviewable_files() {
        let workspace = std::env::temp_dir().join(format!(
            "rust-chatbot-scope-nonreview-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::write(workspace.join("src/lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(workspace.join("src/data.bin"), "binary").unwrap();

        let (files, warnings) = scope::resolve_scope_files(
            &workspace,
            &ReviewScope::Paths(vec![
                "src/lib.rs".to_string(),
                "src/data.bin".to_string(),
            ]),
        )
        .unwrap();

        assert_eq!(files, vec!["src/lib.rs".to_string()]);
        assert!(warnings.is_empty(), "non-reviewable file is silently skipped, not warned");

        let _ = fs::remove_dir_all(workspace);
    }

    // ── Scope resolution: glob with normalized backslashes ───────────

