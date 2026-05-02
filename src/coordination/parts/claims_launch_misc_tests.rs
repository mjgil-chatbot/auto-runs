use super::*;
use crate::sync::{
    role_from_u8, role_to_u8, ROLE_CODE_NONE, ROLE_CODE_ORCHESTRATOR, ROLE_CODE_WORKER,
};

#[test]
pub fn test_role_encoding_none() {
    assert_eq!(role_to_u8(None), ROLE_CODE_NONE);
    assert_eq!(role_from_u8(ROLE_CODE_NONE), None);
}

#[test]
pub fn test_role_encoding_orchestrator() {
    assert_eq!(
        role_to_u8(Some(AgentRole::Orchestrator)),
        ROLE_CODE_ORCHESTRATOR
    );
    assert_eq!(
        role_from_u8(ROLE_CODE_ORCHESTRATOR),
        Some(AgentRole::Orchestrator)
    );
}

#[test]
pub fn test_role_encoding_worker() {
    assert_eq!(role_to_u8(Some(AgentRole::DeepWorker)), ROLE_CODE_WORKER);
    assert_eq!(role_from_u8(ROLE_CODE_WORKER), Some(AgentRole::DeepWorker));
}

#[test]
pub fn test_role_encoding_unknown_code_returns_none() {
    assert_eq!(role_from_u8(99), None);
    assert_eq!(role_from_u8(255), None);
}

#[test]
pub fn test_task_required_role_default_is_none() {
    let now = Utc::now();
    let task = CoordinationTask {
        task_id: "test".to_string(),
        run_id: "run".to_string(),
        title: "Test".to_string(),
        prompt: "Test prompt".to_string(),
        cwd: ".".to_string(),
        status: CoordinationTaskStatus::Queued,
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };
    assert_eq!(task.required_role, None);
}

#[test]
pub fn test_task_required_role_can_be_set() {
    let now = Utc::now();
    let task = CoordinationTask {
        task_id: "test".to_string(),
        run_id: "run".to_string(),
        title: "Test".to_string(),
        prompt: "Test prompt".to_string(),
        cwd: ".".to_string(),
        status: CoordinationTaskStatus::Queued,
        required_role: Some(AgentRole::DeepWorker),
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };
    assert_eq!(task.required_role, Some(AgentRole::DeepWorker));

    let json = serde_json::to_string(&task).unwrap();
    assert!(json.contains("\"required_role\":\"deep_worker\""));

    let decoded: CoordinationTask = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.required_role, Some(AgentRole::DeepWorker));
}

#[test]
pub fn test_task_required_role_orchestrator_round_trip() {
    let now = Utc::now();
    let task = CoordinationTask {
        task_id: "test".to_string(),
        run_id: "run".to_string(),
        title: "Test".to_string(),
        prompt: "Test prompt".to_string(),
        cwd: ".".to_string(),
        status: CoordinationTaskStatus::Queued,
        required_role: Some(AgentRole::Orchestrator),
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };

    let json = serde_json::to_string(&task).unwrap();
    let decoded: CoordinationTask = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.required_role, Some(AgentRole::Orchestrator));
}

#[test]
pub fn test_resolve_autonomous_worker_binary_prefers_current_profile_directory() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let current_exe = root.path().join("target/release/auto-runs");
    let worker = root.path().join("target/release/auto-runs-worker");
    fs::create_dir_all(worker.parent().unwrap()).unwrap();
    fs::write(&current_exe, "").unwrap();
    fs::write(&worker, "").unwrap();

    let resolved = resolve_companion_binary_from_paths(
        &current_exe,
        &root.path().join("auto-runs"),
        "release",
        "auto-runs-worker",
    )
    .unwrap();

    assert_eq!(resolved, worker.canonicalize().unwrap());
}

#[test]
pub fn test_resolve_autonomous_worker_binary_falls_back_to_repo_target_profile() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let current_exe = root.path().join(".cargo/bin/auto-runs");
    let manifest_dir = root.path().join("auto-runs");
    let worker = manifest_dir.join("target/release/auto-runs-worker");
    fs::create_dir_all(current_exe.parent().unwrap()).unwrap();
    fs::create_dir_all(worker.parent().unwrap()).unwrap();
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(&current_exe, "").unwrap();
    fs::write(&worker, "").unwrap();

    let resolved = resolve_companion_binary_from_paths(
        &current_exe,
        &manifest_dir,
        "release",
        "auto-runs-worker",
    )
    .unwrap();

    assert_eq!(resolved, worker.canonicalize().unwrap());
}

#[test]
pub fn test_spawn_provider_task_process_uses_direct_task_handoff_mode() {
    let now = Utc::now();
    let task = CoordinationTask {
        task_id: "task-1".to_string(),
        run_id: "run-1".to_string(),
        title: "Provider task".to_string(),
        prompt: "prompt".to_string(),
        cwd: ".".to_string(),
        assigned_provider: Some("minimax".to_string()),
        status: CoordinationTaskStatus::Queued,
        created_at: now,
        updated_at: now,
        ..CoordinationTask::default()
    };

    let args = autonomous_worker_spawn_args(&task, "minimax", 42);
    assert_eq!(args, ["autonomous", "run-1", "task-1", "minimax", "42"]);
}

#[test]
pub fn test_resolve_autonomous_worker_binary_rejects_profile_mismatch() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let current_exe = root.path().join(".cargo/bin/auto-runs");
    let manifest_dir = root.path().join("auto-runs");
    let mismatch = manifest_dir.join("target/debug/auto-runs-worker");
    fs::create_dir_all(current_exe.parent().unwrap()).unwrap();
    fs::create_dir_all(mismatch.parent().unwrap()).unwrap();
    fs::create_dir_all(&manifest_dir).unwrap();
    fs::write(&current_exe, "").unwrap();
    fs::write(&mismatch, "").unwrap();

    let err = resolve_companion_binary_from_paths(
        &current_exe,
        &manifest_dir,
        "release",
        "auto-runs-worker",
    )
    .unwrap_err();

    assert!(err
        .to_string()
        .contains("mixed debug/release launches are not allowed"));
}

#[test]
pub fn test_autonomous_worker_runsc_mounts_include_provider_state_and_clis() {
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    struct RestoreHome(Option<std::ffi::OsString>);
    impl Drop for RestoreHome {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
    let home = tempfile::tempdir().unwrap();
    let _restore_home = RestoreHome(std::env::var_os("HOME"));
    std::env::set_var("HOME", home.path().canonicalize().unwrap());
    let claude_dir = PathBuf::from(config::get_config_dir(ModelProvider::Claude).unwrap());
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(claude_dir.join(".credentials.json"), "{}").unwrap();
    let mounts = autonomous_worker_runsc_mounts();

    assert!(mounts.iter().any(|mount| {
        mount.target
            == ai_providers::sandbox::RunscMountTarget::ContainerHome(PathBuf::from(
                ".rust-chatbot/coordination",
            ))
            && mount.host.ends_with(std::path::Path::new("coordination"))
    }));

    let mcp_json = PathBuf::from(config::mcp_json_path());
    if mcp_json.exists() {
        assert!(mounts.iter().any(|mount| {
            mount.host == mcp_json
                && mount.target == ai_providers::sandbox::RunscMountTarget::SamePath
        }));
    }

    for provider in [
        ModelProvider::Claude,
        ModelProvider::MiniMax,
        ModelProvider::Qwen,
        ModelProvider::Codex,
        ModelProvider::Nvidia,
        ModelProvider::Gemini,
    ] {
        if let Some(config_dir) = config::get_config_dir(provider).map(PathBuf::from) {
            if config_dir.exists() {
                assert!(mounts.iter().any(|mount| {
                    mount.host == config_dir
                        && mount.target == ai_providers::sandbox::RunscMountTarget::SamePath
                }));
            }
        }
    }

    if let Some(claude_dir) = config::get_config_dir(ModelProvider::Claude).map(PathBuf::from) {
        assert!(mounts.iter().any(|mount| {
            mount.host == claude_dir.join(".credentials.json")
                && mount.target
                    == ai_providers::sandbox::RunscMountTarget::ContainerHome(PathBuf::from(
                        ".claude/.credentials.json",
                    ))
        }));
    }

    let codex_sidecar = codex_home_sidecar_dir();
    if codex_sidecar.exists() {
        assert!(mounts.iter().any(|mount| {
            mount.host == codex_sidecar
                && mount.target
                    == ai_providers::sandbox::RunscMountTarget::ContainerHome(PathBuf::from(
                        ".codex",
                    ))
        }));
    }

    let gemini_dir = config::get_config_dir(ModelProvider::Gemini)
        .map(PathBuf::from)
        .unwrap_or_else(|| config::canonical_shared_root_dir().join(".gemini"));
    if gemini_dir.join("oauth_creds.json").exists() {
        assert!(mounts.iter().any(|mount| {
            mount.host == gemini_dir.join("oauth_creds.json")
                && mount.target
                    == ai_providers::sandbox::RunscMountTarget::ContainerHome(PathBuf::from(
                        ".gemini/oauth_creds.json",
                    ))
        }));
    }

    if let Some(qwen_dir) = config::get_config_dir(ModelProvider::Qwen).map(PathBuf::from) {
        if qwen_dir.join("oauth_creds.json").exists() {
            assert!(mounts.iter().any(|mount| {
                mount.host == qwen_dir.join("oauth_creds.json")
                    && mount.target
                        == ai_providers::sandbox::RunscMountTarget::ContainerHome(PathBuf::from(
                            ".qwen/oauth_creds.json",
                        ))
            }));
        }
    }

    for cli_name in [
        "claude".to_string(),
        "gemini".to_string(),
        config::qwen_code_cli_path(),
        config::codex_cli_path(),
    ] {
        for expected_dir in ai_providers::sandbox::program_directory_mounts(&cli_name) {
            assert!(mounts.iter().any(|mount| {
                mount.host == expected_dir
                    && mount.target == ai_providers::sandbox::RunscMountTarget::SamePath
            }));
        }
    }
}

#[test]
pub fn test_legacy_task_without_required_role_deserializes() {
    let legacy_json = r#"{
        "task_id": "test",
        "run_id": "run",
        "title": "Test",
        "prompt": "Test prompt",
        "cwd": ".",
        "status": "queued",
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z"
    }"#;

    let task: CoordinationTask = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(task.required_role, None);
    assert_eq!(task.task_id, "test");
}
