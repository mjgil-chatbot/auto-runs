fn run_git_for_test(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} should succeed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn collect_relative_files(
    root: &std::path::Path,
    current: &std::path::Path,
    output: &mut Vec<std::path::PathBuf>,
) {
    for entry in std::fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap();
        if relative
            .components()
            .next()
            .and_then(|component| component.as_os_str().to_str())
            .is_some_and(|component| component == ".git" || component == "target")
        {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            collect_relative_files(root, &path, output);
            continue;
        }
        if metadata.is_file() {
            output.push(relative.to_path_buf());
        }
    }
}

fn assert_rust_scaffold_matches(workspace: &std::path::Path) {
    let scaffold_root_str = super::workspace_ready::rust_scaffold_root();
    let scaffold_root = std::path::Path::new(&scaffold_root_str);
    if !scaffold_root.is_dir() {
        // Skip test if scaffold is not available
        return;
    }
    let mut relative_paths = Vec::new();
    collect_relative_files(scaffold_root, scaffold_root, &mut relative_paths);

    for relative_path in relative_paths {
        let source = scaffold_root.join(&relative_path);
        let destination = workspace.join(&relative_path);
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            std::fs::read(&source).unwrap(),
            "{} should match the scaffold contents",
            relative_path.display(),
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let source_mode = std::fs::metadata(&source).unwrap().permissions().mode();
            let mode = std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o111 != 0,
                source_mode & 0o111 != 0,
                "{} executable bit should match the scaffold",
                relative_path.display()
            );
        }
    }
}

fn assert_rust_scaffold_workspace_settings(workspace: &std::path::Path) {
    let cargo_toml = std::fs::read_to_string(workspace.join("Cargo.toml")).unwrap();
    assert!(cargo_toml.contains("[workspace]"));
    assert!(cargo_toml.contains("\"apps/github-branch-fix\""));
    assert!(cargo_toml.contains("\"crates/scaffold-allocator\""));
    assert!(cargo_toml.contains("cap = \"0.1.2\""));
    assert!(cargo_toml.contains("[profile.release]"));
    assert!(cargo_toml.contains("opt-level = 1"));
    assert!(cargo_toml.contains("[profile.ship]"));
    assert!(cargo_toml.contains("lto = true"));
    assert!(cargo_toml.contains("[profile.coverage]"));

    let cargo_config = std::fs::read_to_string(workspace.join(".cargo/config.toml")).unwrap();
    assert!(cargo_config.contains("[target.x86_64-unknown-linux-gnu]"));
    assert!(cargo_config.contains("linker = \"clang\""));
    assert!(cargo_config.contains("link-arg=-fuse-ld=mold"));
    assert!(cargo_config.contains("target-dir = \"target/release\""));
    assert!(cargo_config.contains("rustc-wrapper = \"sccache\""));
    assert!(cargo_config.contains("jobs = 1"));

    assert_eq!(
        std::fs::read_to_string(workspace.join(".gitignore")).unwrap(),
        "target\n**/*.rs.bk\n",
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("rust-toolchain.toml")).unwrap(),
        "[toolchain]\nchannel = \"1.94.1\"\ncomponents = [\"rustfmt\"]\nprofile = \"minimal\"\n",
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("rustfmt.toml")).unwrap(),
        "edition = \"2021\"\nstyle_edition = \"2021\"\nmax_width = 100\nnewline_style = \"Unix\"\nuse_small_heuristics = \"Default\"\n",
    );
}

fn seed_existing_github_repo(
    remote_root: &std::path::Path,
    repo_name: &str,
    extra_auto_runs_branches: &[&str],
) -> std::path::PathBuf {
    let remote_path = remote_root.join(format!("{repo_name}.git"));
    std::fs::create_dir_all(&remote_path).unwrap();
    run_git_for_test(&remote_path, &["init", "--bare"]);

    let seed_dir = tempfile::TempDir::new().unwrap();
    run_git_for_test(seed_dir.path(), &["init", "-b", "main"]);
    run_git_for_test(seed_dir.path(), &["config", "user.name", "Test User"]);
    run_git_for_test(
        seed_dir.path(),
        &["config", "user.email", "test@example.com"],
    );
    std::fs::write(seed_dir.path().join("README.md"), "# Seed\n").unwrap();
    run_git_for_test(seed_dir.path(), &["add", "README.md"]);
    run_git_for_test(seed_dir.path(), &["commit", "-m", "seed"]);
    run_git_for_test(
        seed_dir.path(),
        &["remote", "add", "origin", remote_path.to_str().unwrap()],
    );
    run_git_for_test(seed_dir.path(), &["push", "-u", "origin", "main"]);

    for branch in extra_auto_runs_branches {
        run_git_for_test(seed_dir.path(), &["checkout", "--orphan", branch]);
        std::fs::write(
            seed_dir.path().join("branch.txt"),
            format!("branch {branch}\n"),
        )
        .unwrap();
        run_git_for_test(seed_dir.path(), &["add", "-A"]);
        run_git_for_test(seed_dir.path(), &["commit", "-m", branch]);
        run_git_for_test(
            seed_dir.path(),
            &["push", "-u", "origin", &format!("HEAD:refs/heads/{branch}")],
        );
    }

    remote_path
}

#[test]
fn worker_dispatch_supports_new_autonomous_registry_providers() {
    assert_eq!(
        resolve_autonomous_provider("nemotron").unwrap(),
        ModelProvider::Nvidia
    );
}

#[test]
fn append_runsc_ready_event_records_explicit_recent_event() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.execution_environment = AutonomousExecutionEnvironment::Runsc;
    run.runsc_workspace_includes = vec!["instructions".to_string(), "plan.txt".to_string()];
    let task = fixture.make_task(
        &run,
        "runsc-ready",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Running,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    append_runsc_ready_event(&run, &task, "codex").unwrap();

    let events = load_events(&run.run_id, None, None).unwrap();
    let event = events.last().unwrap();
    assert_eq!(event.event_type, "task_runsc_ready");
    assert_eq!(event.provider.as_deref(), Some("codex"));
    assert!(event.message.contains("Runsc started correctly"));
    assert!(event
        .message
        .contains("Explicit includes: instructions, plan.txt"));
}

#[test]
fn ensure_task_workspace_exists_records_explicit_recent_event() {
    let fixture = CoordinationTestFixture::new();
    let missing_workspace = fixture.workspace.path().join("provider-missing");

    let mut run = fixture.make_run(AutonomousWorkflowMode::SingleProvider);
    run.workspace_path = missing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "workspace-create",
        CoordinationTaskKind::PlanFlatTasks,
        Some("codex"),
        "Plan non-overlapping implementation tasks from design doc",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert!(missing_workspace.is_dir());

    let events = load_events(&run.run_id, None, None).unwrap();
    let event = events.last().unwrap();
    assert_eq!(event.event_type, "task_workspace_created");
    assert_eq!(event.task_id.as_deref(), Some(task.task_id.as_str()));
    assert_eq!(event.provider.as_deref(), Some("codex"));
    assert!(event
        .message
        .contains("Created missing workspace directory"));
    assert!(event.message.contains(&run.workspace_path));
}

#[test]
fn ensure_task_workspace_exists_initializes_lightweight_git_workspace() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let missing_workspace = fixture.workspace.path().join("lightweight-missing");

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "workspace-create-lightweight",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert!(missing_workspace.is_dir());
    assert!(missing_workspace.join(".git").is_dir());

    let top_level = run_git_for_test(&missing_workspace, &["rev-parse", "--show-toplevel"]);
    let origin = run_git_for_test(&missing_workspace, &["remote", "get-url", "origin"]);
    let remotes = run_git_for_test(&missing_workspace, &["remote"]);

    assert_eq!(top_level, missing_workspace.display().to_string());
    assert!(std::path::Path::new(&origin).is_dir());
    assert!(origin.starts_with(remote_root.path().to_str().unwrap()));
    assert_eq!(remotes, "origin");

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_workspace_created"));
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_git_initialized"
            && event
                .message
                .contains("Initialized lightweight git repository")
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_github_remote_created"
            && event.message.contains("remote `origin`")
    }));
}

#[test]
fn worker_poll_claims_skip_builtin_lightweight_survey_tasks() {
    let fixture = CoordinationTestFixture::new();
    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Zig,
    );
    let survey = fixture.make_task(
        &run,
        "lightweight-survey-builtin",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&survey));

    let claimed = claim_next_runnable_provider_task(&run.run_id, 77, "codex").unwrap();

    assert!(
        claimed.is_none(),
        "poll workers must not claim builtin survey tasks"
    );
    let persisted = load_task(&run.run_id, &survey.task_id).unwrap();
    assert_eq!(persisted.status, CoordinationTaskStatus::Queued);
    assert!(load_claim(&run.run_id, &survey.task_id).is_err());
}

#[test]
fn ensure_task_workspace_exists_applies_rust_repo_configuration() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let missing_workspace = fixture.workspace.path().join("lightweight-rust-config");

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let task = fixture.make_task(
        &run,
        "workspace-create-lightweight-rust",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_rust_scaffold_matches(&missing_workspace);
    assert_rust_scaffold_workspace_settings(&missing_workspace);
    assert_eq!(
        run_git_for_test(&missing_workspace, &["config", "--get", "core.hooksPath"]),
        ".githooks"
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_repo_configured"
            && event.message.contains("Applied Rust repo configuration")
    }));
}

#[test]
fn ensure_task_workspace_exists_existing_repo_uses_first_isolated_auto_runs_branch() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let repo_name = "lightweight-existing-remote";
    let missing_workspace = fixture.workspace.path().join(repo_name);
    let remote_path = seed_existing_github_repo(remote_root.path(), repo_name, &[]);

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "workspace-create-lightweight-existing-remote",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_eq!(
        run_git_for_test(&missing_workspace, &["remote", "get-url", "origin"]),
        remote_path.display().to_string()
    );
    assert_eq!(
        run_git_for_test(&missing_workspace, &["symbolic-ref", "--short", "HEAD"]),
        "auto-runs-1"
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_git_branch_selected"
            && event.message.contains("auto-runs-1")
    }));
}

#[test]
fn ensure_task_workspace_exists_existing_repo_increments_isolated_auto_runs_branch_suffix() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let repo_name = "lightweight-existing-remote-next";
    let missing_workspace = fixture.workspace.path().join(repo_name);
    seed_existing_github_repo(
        remote_root.path(),
        repo_name,
        &["auto-runs-1", "auto-runs-2"],
    );

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "workspace-create-lightweight-existing-remote-next",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_eq!(
        run_git_for_test(&missing_workspace, &["symbolic-ref", "--short", "HEAD"]),
        "auto-runs-3"
    );
}

#[test]
fn ensure_task_workspace_exists_reuses_existing_github_origin_for_branch_worktree_names() {
    let fixture = CoordinationTestFixture::new();
    let _gh_guard = EnvVarGuard::set("AUTO_RUNS_GH_BIN", "/definitely/missing/gh");
    let existing_workspace = fixture
        .workspace
        .path()
        .join("rustc-wasm-builder-enscripten");

    std::fs::create_dir_all(&existing_workspace).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "enscripten"]);
    run_git_for_test(&existing_workspace, &["config", "user.name", "Test User"]);
    run_git_for_test(
        &existing_workspace,
        &["config", "user.email", "test@example.com"],
    );
    run_git_for_test(
        &existing_workspace,
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:mjgil/rustc-wasm-builder.git",
        ],
    );

    let mut run = fixture.make_run(AutonomousWorkflowMode::GithubBranchFix);
    run.workspace_path = existing_workspace.display().to_string();
    let task = fixture.make_task(
        &run,
        "workspace-existing-branch-fix",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_eq!(
        run_git_for_test(&existing_workspace, &["remote", "get-url", "origin"]),
        "git@github.com:mjgil/rustc-wasm-builder.git"
    );
    assert_eq!(run_git_for_test(&existing_workspace, &["remote"]), "origin");
}

#[test]
fn ensure_task_workspace_exists_does_not_overwrite_existing_rust_workspace() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let existing_workspace = fixture.workspace.path().join("lightweight-rust-existing");

    std::fs::create_dir_all(existing_workspace.join(".cargo")).unwrap();
    std::fs::create_dir_all(existing_workspace.join(".githooks")).unwrap();
    std::fs::create_dir_all(existing_workspace.join("scripts")).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "main"]);
    std::fs::write(
        existing_workspace.join(".gitignore"),
        "local override for gitignore\n",
    )
    .unwrap();
    std::fs::write(
        existing_workspace.join(".cargo/config.toml"),
        "[build]\ntarget-dir = \"/tmp/local-target\"\n",
    )
    .unwrap();
    std::fs::write(
        existing_workspace.join("rust-toolchain.toml"),
        "channel = \"nightly\"\n",
    )
    .unwrap();
    std::fs::write(existing_workspace.join("rustfmt.toml"), "max_width = 80\n").unwrap();
    std::fs::write(
        existing_workspace.join(".githooks/pre-commit"),
        "#!/bin/sh\necho local hook\n",
    )
    .unwrap();
    std::fs::write(
        existing_workspace.join("scripts/install-git-hooks.sh"),
        "#!/bin/sh\necho local installer\n",
    )
    .unwrap();

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = existing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let task = fixture.make_task(
        &run,
        "workspace-reapply-lightweight-rust",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    let gitignore = std::fs::read_to_string(existing_workspace.join(".gitignore")).unwrap();
    assert!(gitignore.contains("local override for gitignore"));
    assert!(gitignore.lines().any(|line| line.trim() == "target"));
    assert_eq!(
        std::fs::read_to_string(existing_workspace.join(".cargo/config.toml")).unwrap(),
        "[build]\ntarget-dir = \"/tmp/local-target\"\n",
    );
    assert_eq!(
        std::process::Command::new("git")
            .args(["config", "--get", "core.hooksPath"])
            .current_dir(&existing_workspace)
            .output()
            .unwrap()
            .status
            .success(),
        false
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(!events.iter().any(|event| {
        event.event_type == "task_workspace_repo_configured"
            && event.message.contains("Applied Rust repo configuration")
    }));
}

#[test]
fn ensure_task_workspace_exists_treats_docs_only_rust_workspace_as_fresh() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let existing_workspace = fixture.workspace.path().join("lightweight-rust-docs-only");

    std::fs::create_dir_all(existing_workspace.join("docs")).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "main"]);
    std::fs::write(existing_workspace.join("docs/notes.md"), "# Notes\n").unwrap();

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = existing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let task = fixture.make_task(
        &run,
        "workspace-docs-only-lightweight-rust",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_rust_scaffold_matches(&existing_workspace);
    assert_rust_scaffold_workspace_settings(&existing_workspace);
    assert_eq!(
        run_git_for_test(&existing_workspace, &["config", "--get", "core.hooksPath"]),
        ".githooks"
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_repo_configured"
            && event.message.contains("Applied Rust repo configuration")
    }));
}

#[test]
fn ensure_task_workspace_exists_repairs_target_permissions_and_gitignore() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let existing_workspace = fixture
        .workspace
        .path()
        .join("lightweight-permission-repair");

    std::fs::create_dir_all(existing_workspace.join("src")).unwrap();
    std::fs::create_dir_all(existing_workspace.join("target/release/.fingerprint")).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "main"]);
    std::fs::write(
        existing_workspace.join("src/lib.rs"),
        "pub fn sample() {}\n",
    )
    .unwrap();
    std::fs::write(existing_workspace.join(".gitignore"), "dist\n").unwrap();

    let fingerprint = existing_workspace.join("target/release/.fingerprint/build");
    std::fs::write(&fingerprint, "stale build output\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&fingerprint).unwrap().permissions();
        permissions.set_mode(0o400);
        std::fs::set_permissions(&fingerprint, permissions).unwrap();
    }

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = existing_workspace.display().to_string();
    run.notes = lightweight_notes_with_settings(
        "",
        "docs/autonomous/lightweight-tasks.md",
        "codex",
        &["codex".to_string()],
    );
    let task = fixture.make_task(
        &run,
        "workspace-repair-target-permissions",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());

    let gitignore = std::fs::read_to_string(existing_workspace.join(".gitignore")).unwrap();
    assert!(gitignore.contains("dist"));
    assert!(gitignore.lines().any(|line| line.trim() == "target"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&fingerprint)
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(mode & 0o200, 0);
    }

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_workspace_gitignore_updated"));
    assert!(events
        .iter()
        .any(|event| event.event_type == "task_workspace_permissions_repaired"));
}

#[test]
fn ensure_task_workspace_exists_bootstraps_rust_workspace_with_expected_scaffold_settings() {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let missing_workspace = fixture
        .workspace
        .path()
        .join("lightweight-rust-scaffold-settings");

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = missing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let task = fixture.make_task(
        &run,
        "workspace-create-lightweight-rust-settings",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_rust_scaffold_workspace_settings(&missing_workspace);
}

#[test]
fn ensure_task_workspace_exists_bootstraps_docs_only_rust_workspace_with_expected_scaffold_settings(
) {
    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let existing_workspace = fixture
        .workspace
        .path()
        .join("lightweight-rust-docs-only-settings");

    std::fs::create_dir_all(existing_workspace.join("docs")).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "main"]);
    std::fs::write(existing_workspace.join("docs/design.md"), "# Design\n").unwrap();

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = existing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Rust,
    );
    let task = fixture.make_task(
        &run,
        "workspace-docs-only-lightweight-rust-settings",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert_rust_scaffold_workspace_settings(&existing_workspace);
}

#[test]
fn ensure_task_workspace_exists_reapplies_zig_repo_configuration_for_existing_workspace() {
    let zig_scaffold_str = super::workspace_ready::zig_scaffold_root();
    let zig_scaffold = std::path::Path::new(&zig_scaffold_str);
    if !zig_scaffold.join("lib/compiler.zig").is_file() {
        return;
    }

    let fixture = CoordinationTestFixture::new();
    let (_gh_dir, _remote_root, _gh_guard, _remote_guard) = install_fake_gh_cli();
    let existing_workspace = fixture.workspace.path().join("lightweight-zig-existing");

    std::fs::create_dir_all(existing_workspace.join(".githooks")).unwrap();
    std::fs::create_dir_all(existing_workspace.join("scripts")).unwrap();
    run_git_for_test(&existing_workspace, &["init", "-b", "main"]);
    std::fs::write(
        existing_workspace.join(".githooks/pre-commit"),
        "#!/usr/bin/env bash\necho pre-commit: running cargo test\ncargo test\n",
    )
    .unwrap();
    std::fs::write(
        existing_workspace.join("scripts/install-git-hooks.sh"),
        "#!/usr/bin/env bash\necho local installer\n",
    )
    .unwrap();

    let mut run = fixture.make_run(AutonomousWorkflowMode::LightweightImplementation);
    run.workspace_path = existing_workspace.display().to_string();
    run.notes = lightweight_notes_with_repo_configuration(
        &lightweight_notes_with_settings(
            "",
            "docs/autonomous/lightweight-tasks.md",
            "codex",
            &["codex".to_string()],
        ),
        LightweightRepoConfiguration::Zig,
    );
    let task = fixture.make_task(
        &run,
        "workspace-reapply-lightweight-zig",
        CoordinationTaskKind::SurveyWorkspace,
        Some("codex"),
        "Survey workspace",
        &["**"],
        &[],
        &[],
        CoordinationTaskStatus::Queued,
    );
    fixture.persist_run_with_tasks(&run, std::slice::from_ref(&task));

    assert!(!ensure_task_workspace_exists(&run, &task, Some("codex")).unwrap());
    assert!(existing_workspace.join(".gitignore").is_file());
    let hook = std::fs::read_to_string(existing_workspace.join(".githooks/pre-commit")).unwrap();
    assert!(hook.contains("zig fmt --check ."));
    assert!(hook.contains("zig build test"));
    let installer =
        std::fs::read_to_string(existing_workspace.join("scripts/install-git-hooks.sh")).unwrap();
    assert!(installer.contains("core.hooksPath .githooks"));
    assert_eq!(
        run_git_for_test(&existing_workspace, &["config", "--get", "core.hooksPath"]),
        ".githooks"
    );

    let events = load_events(&run.run_id, None, None).unwrap();
    assert!(events.iter().any(|event| {
        event.event_type == "task_workspace_repo_configured"
            && event.message.contains("Applied Zig repo configuration")
    }));
}
