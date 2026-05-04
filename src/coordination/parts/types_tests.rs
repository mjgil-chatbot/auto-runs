use super::*;
use std::{collections::HashSet, fs};

#[test]
pub fn provider_capability_registry_file_deserializes() {
    let raw = fs::read_to_string(PROVIDER_CAPABILITY_REGISTRY_PATH)
        .expect("provider capability registry should exist");
    let registry: ProviderCapabilityRegistry =
        serde_json::from_str(&raw).expect("provider capability registry should deserialize");

    assert!(
        !registry.providers.is_empty(),
        "provider capability registry should define at least one provider"
    );

    let provider_ids: HashSet<&str> = registry
        .providers
        .iter()
        .map(|profile| profile.provider_id.as_str())
        .collect();
    assert_eq!(
        provider_ids.len(),
        registry.providers.len(),
        "provider ids should be unique"
    );

    for profile in &registry.providers {
        assert!(
            !profile.provider_id.trim().is_empty(),
            "provider ids should not be blank"
        );
        assert!(
            !profile.supported_categories.is_empty(),
            "{} should support at least one category",
            profile.provider_id
        );
        assert!(
            profile.max_concurrency > 0,
            "{} should advertise concurrency",
            profile.provider_id
        );
        assert!(
            !profile.fallback_chain.is_empty(),
            "{} should advertise fallback providers",
            profile.provider_id
        );
        for fallback in &profile.fallback_chain {
            assert!(
                provider_ids.contains(fallback.as_str()),
                "{} fallback {} should exist in the registry",
                profile.provider_id,
                fallback
            );
            assert_ne!(
                fallback, &profile.provider_id,
                "{} should not fall back to itself",
                profile.provider_id
            );
        }
    }
}

#[test]
pub fn coordination_task_round_trips_capability_metadata() {
    let task = CoordinationTask {
        task_id: "task-1".to_string(),
        run_id: "run-1".to_string(),
        parent_task_id: Some("parent-1".to_string()),
        kind: CoordinationTaskKind::GeneratedWriteTask,
        assigned_provider: Some("codex".to_string()),
        task_category: Some(TaskCategory::DeepWrite),
        provider_hint: Some("claude".to_string()),
        required_role: Some(AgentRole::DeepWorker),
        iteration: Some(2),
        title: "Implement schema".to_string(),
        prompt: "Add capability metadata".to_string(),
        cwd: "/tmp".to_string(),
        allowed_tools: vec!["filesystem".to_string(), "edit".to_string()],
        read_scope: vec!["src/**".to_string()],
        write_scope: vec!["src/coordination/parts/types.rs".to_string()],
        acceptance_checks: vec!["serde round-trip succeeds".to_string()],
        review_metadata: None,
        dependencies: vec!["task-0".to_string()],
        execution_generation: default_execution_generation(),
        dependency_generation: None,
        attempt_count: 1,
        max_attempts: 4,
        publish_attempt_count: 0,
        estimated_minutes: Some(25),
        status: CoordinationTaskStatus::Queued,
        claimed_by: Some(42),
        artifact_summary: Some("pending".to_string()),
        failure_kind: None,
        recovery_action: None,
        created_at: DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339("2026-04-05T12:01:00Z")
            .unwrap()
            .with_timezone(&Utc),
    };

    let json = serde_json::to_string(&task).expect("task should serialize");
    assert!(json.contains("\"task_category\":\"deep_write\""));
    assert!(json.contains("\"provider_hint\":\"claude\""));
    assert!(json.contains("\"required_role\":\"deep_worker\""));

    let decoded: CoordinationTask = serde_json::from_str(&json).expect("task should deserialize");
    assert_eq!(decoded.task_category, Some(TaskCategory::DeepWrite));
    assert_eq!(decoded.provider_hint.as_deref(), Some("claude"));
    assert_eq!(decoded.required_role, Some(AgentRole::DeepWorker));
    assert_eq!(decoded.assigned_provider.as_deref(), Some("codex"));
    assert_eq!(decoded.estimated_minutes, Some(25));
}

#[test]
pub fn task_claim_round_trips_worker_lifecycle_metadata() {
    let timestamp = DateTime::parse_from_rfc3339("2026-04-05T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let claim = TaskClaim {
        run_id: "run-1".to_string(),
        task_id: "task-1".to_string(),
        provider: "codex".to_string(),
        instance_pid: 222,
        launcher_pid: Some(111),
        lifecycle_phase: ClaimLifecyclePhase::TaskExecutionStarted,
        worker_pid: Some(222),
        claimed_at: timestamp,
        worker_spawned_at: Some(timestamp),
        worker_adopted_at: Some(timestamp),
        execution_started_at: Some(timestamp),
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: timestamp,
    };

    let json = serde_json::to_string(&claim).expect("claim should serialize");
    assert!(json.contains("\"lifecycle_phase\":\"task_execution_started\""));
    assert!(json.contains("\"worker_pid\":222"));

    let decoded: TaskClaim = serde_json::from_str(&json).expect("claim should deserialize");
    assert_eq!(
        decoded.lifecycle_phase,
        ClaimLifecyclePhase::TaskExecutionStarted
    );
    assert_eq!(decoded.worker_pid, Some(222));
    assert_eq!(decoded.launcher_pid, Some(111));
}

#[test]
pub fn coordination_task_deserializes_legacy_json_without_capability_metadata() {
    let legacy = r#"{
        "task_id": "task-legacy",
        "run_id": "run-legacy",
        "parent_task_id": null,
        "kind": "survey_workspace",
        "assigned_provider": "gemini",
        "iteration": null,
        "title": "Survey workspace",
        "prompt": "Inspect the repository",
        "cwd": ".",
        "allowed_tools": [],
        "read_scope": [],
        "write_scope": [],
        "acceptance_checks": [],
        "review_metadata": null,
        "dependencies": [],
        "attempt_count": 0,
        "max_attempts": 3,
        "status": "queued",
        "claimed_by": null,
        "artifact_summary": null,
        "created_at": "2026-04-05T12:00:00Z",
        "updated_at": "2026-04-05T12:00:00Z"
    }"#;

    let task: CoordinationTask =
        serde_json::from_str(legacy).expect("legacy task json should deserialize");

    assert_eq!(task.kind, CoordinationTaskKind::SurveyWorkspace);
    assert_eq!(task.assigned_provider.as_deref(), Some("gemini"));
    assert_eq!(task.task_category, None);
    assert_eq!(task.provider_hint, None);
    assert_eq!(task.required_role, None);
}

#[test]
pub fn coordination_task_deserializes_worker_role_alias() {
    let legacy = r#"{
        "task_id": "task-role-alias",
        "run_id": "run-role-alias",
        "parent_task_id": null,
        "kind": "generated_write_task",
        "assigned_provider": "codex",
        "task_category": "deep_write",
        "provider_hint": "claude",
        "required_role": "worker",
        "iteration": 1,
        "title": "Implement change",
        "prompt": "Patch the code",
        "cwd": ".",
        "allowed_tools": [],
        "read_scope": [],
        "write_scope": [],
        "acceptance_checks": [],
        "review_metadata": null,
        "dependencies": [],
        "attempt_count": 0,
        "max_attempts": 3,
        "status": "queued",
        "claimed_by": null,
        "artifact_summary": null,
        "created_at": "2026-04-05T12:00:00Z",
        "updated_at": "2026-04-05T12:00:00Z"
    }"#;

    let task: CoordinationTask =
        serde_json::from_str(legacy).expect("legacy worker role should deserialize");

    assert_eq!(task.required_role, Some(AgentRole::DeepWorker));
}
