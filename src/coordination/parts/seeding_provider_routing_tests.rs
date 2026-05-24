mod seeding_provider_routing_tests {
    use super::*;

    fn make_test_run() -> CoordinationRun {
        CoordinationRun {
            run_id: "test".to_string(),
            root_chat_id: "test".to_string(),
            workspace_path: "/tmp".to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "claude".to_string(),
            goal: "test".to_string(),
            workflow_mode: AutonomousWorkflowMode::DesignConsensus,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: String::new(),
            max_iterations: 1,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 4,
            allow_write_tasks: true,
            require_review_before_apply: true,
            auto_retry_failed_tasks: false,
            notes: String::new(),
            review_scope: None,
            review_dimensions: None,
            review_fix_mode: None,
            council_turn_policy: None,
            council_max_turns: None,
            provider_cooldowns: BTreeMap::new(),
            task_ids: Vec::new(),
           processed_mutation_batch_ids: BTreeSet::new(),
        }
    }

    #[test]
    fn resolve_design_draft_providers_returns_nonempty() {
        let run = make_test_run();
        let providers = resolve_design_draft_providers(&run);
        assert!(!providers.is_empty(), "must resolve at least one provider");
    }

    #[test]
    fn resolve_design_draft_providers_prefers_design_capable() {
        let run = make_test_run();
        let providers = resolve_design_draft_providers(&run);
        assert!(
            providers.contains(&"claude".to_string()),
            "claude should be in design providers"
        );
    }

    #[test]
    fn resolve_design_draft_providers_skips_cooled_down() {
        let mut run = make_test_run();
        run.provider_cooldowns.insert(
            "claude".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "rate limit".to_string(),
            },
        );
        let providers = resolve_design_draft_providers(&run);
        assert!(
            !providers.contains(&"claude".to_string()),
            "cooled-down provider must be excluded"
        );
        assert!(
            !providers.is_empty(),
            "fallback providers must be available"
        );
    }

    #[test]
    fn category_routing_selects_preferred_provider() {
        let run = make_test_run();

        let plan_provider = resolve_provider_for_category(&run, &TaskCategory::Plan, None);
        assert_eq!(plan_provider.as_deref(), Some("codex"));

        let design_provider = resolve_provider_for_category(&run, &TaskCategory::Design, None);
        assert_eq!(design_provider.as_deref(), Some("claude"));
    }

    #[test]
    fn category_routing_respects_hint() {
        let run = make_test_run();
        let provider = resolve_provider_for_category(&run, &TaskCategory::Plan, Some("gemini"));
        assert_eq!(provider.as_deref(), Some("gemini"));
    }

    #[test]
    fn category_routing_uses_category_safe_fallback_chain_when_hint_is_cooled_down() {
        let mut run = make_test_run();
        run.provider_cooldowns.insert(
            "claude".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "rate limit".to_string(),
            },
        );
        run.provider_cooldowns.insert(
            "codex".to_string(),
            ProviderCooldown {
                unavailable_until: Utc::now() + chrono::Duration::hours(1),
                reason: "rate limit".to_string(),
            },
        );

        let provider = resolve_provider_for_category(&run, &TaskCategory::Design, Some("claude"));
        let registry = load_capability_registry();
        let selected = provider.expect("design fallback provider should be available");
        assert_ne!(selected, "claude");
        assert_ne!(selected, "codex");
        assert!(
            registry
                .providers
                .iter()
                .find(|profile| profile.provider_id == selected)
                .map(|profile| profile.supported_categories.contains(&TaskCategory::Design))
                .unwrap_or(false),
            "fallback provider should support the requested design category"
        );
    }

    #[test]
    fn prompt_family_applies_mechanics_suffix() {
        let codex_prompt = apply_prompt_family("codex", "Do something.");
        assert!(codex_prompt.contains("Follow these steps precisely"));

        let claude_prompt = apply_prompt_family("claude", "Do something.");
        assert!(!claude_prompt.contains("Follow these steps precisely"));
        assert_eq!(claude_prompt, "Do something.");
    }
}
