use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::MutexGuard;

use tempfile::TempDir;

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn install_fake_gh_cli() -> (TempDir, TempDir, EnvVarGuard, EnvVarGuard) {
    let script_dir = TempDir::new().unwrap();
    let remote_root = TempDir::new().unwrap();
    let script_path = script_dir.path().join("fake-gh.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
set -eu

if [ "${1:-}" = "--version" ]; then
  echo "gh version test"
  exit 0
fi

if [ "${1:-}" = "auth" ]; then
  shift
  subcommand=${1:-}
  shift || true
  case "$subcommand" in
    status)
      if [ "${AUTO_RUNS_TEST_GH_ACTIVE_UNSUPPORTED:-ok}" = "fail" ] && [ "${1:-}" = "--active" ]; then
        echo "unknown flag: --active" >&2
        exit 1
      fi
      if [ "${AUTO_RUNS_TEST_GH_AUTH:-ok}" = "fail" ]; then
        echo "gh auth status: not logged in" >&2
        exit 1
      fi
      echo "github.com"
      exit 0
      ;;
    *)
      echo "unsupported gh auth subcommand: $subcommand" >&2
      exit 1
      ;;
  esac
fi

if [ "${1:-}" = "run" ]; then
  shift
  subcommand=${1:-}
  shift || true
  case "$subcommand" in
    list)
      if [ "${AUTO_RUNS_TEST_GH_RUN_LIST_EXIT_CODE:-0}" != "0" ]; then
        echo "${AUTO_RUNS_TEST_GH_RUN_LIST_STDERR:-gh run list failed}" >&2
        exit "${AUTO_RUNS_TEST_GH_RUN_LIST_EXIT_CODE}"
      fi
      printf '%s\n' "${AUTO_RUNS_TEST_GH_RUN_LIST_STDOUT:-[]}"
      exit 0
      ;;
    *)
      echo "unsupported gh run subcommand: $subcommand" >&2
      exit 1
      ;;
  esac
fi

if [ "${1:-}" != "repo" ]; then
  echo "unsupported gh command: $*" >&2
  exit 1
fi
shift

subcommand=${1:-}
shift || true
remote_root=${AUTO_RUNS_TEST_GH_REMOTE_ROOT:?}

case "$subcommand" in
  create)
    repo_name=${1:?}
    shift || true
    remote_name=origin
    while [ $# -gt 0 ]; do
      case "$1" in
        --remote=*)
          remote_name=${1#--remote=}
          ;;
        --remote)
          shift
          remote_name=${1:?}
          ;;
      esac
      shift || true
    done
    remote_path="$remote_root/$repo_name.git"
    mkdir -p "$remote_root"
    if [ -d "$remote_path" ]; then
      echo "repository already exists" >&2
      exit 1
    fi
    git init --bare "$remote_path" >/dev/null 2>&1
    git -C "$remote_path" symbolic-ref HEAD refs/heads/main >/dev/null 2>&1 || true
    git remote add "$remote_name" "$remote_path"
    printf '%s\n' "$remote_path"
    ;;
  view)
    repo_name=${1:?}
    remote_path="$remote_root/$repo_name.git"
    if [ ! -d "$remote_path" ]; then
      exit 1
    fi
    printf '%s\n' "$remote_path"
    ;;
  *)
    echo "unsupported gh repo subcommand: $subcommand" >&2
    exit 1
    ;;
esac
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).unwrap();
    }
    let gh_guard = EnvVarGuard::set("AUTO_RUNS_GH_BIN", &script_path);
    let remote_guard = EnvVarGuard::set("AUTO_RUNS_TEST_GH_REMOTE_ROOT", remote_root.path());
    (script_dir, remote_root, gh_guard, remote_guard)
}

struct CoordinationTestFixture {
    _env_lock: MutexGuard<'static, ()>,
    _home: EnvVarGuard,
    _home_dir: TempDir,
    _rust_scaffold: EnvVarGuard,
    _zig_scaffold: EnvVarGuard,
    workspace: TempDir,
}

impl CoordinationTestFixture {
    fn new() -> Self {
        Self::named("coordination")
    }

    fn named(_label: &str) -> Self {
        let env_lock = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let temporary_root = std::env::temp_dir().canonicalize().unwrap();
        let home = TempDir::new_in(&temporary_root).unwrap();
        let workspace = TempDir::new_in(&temporary_root).unwrap();
        let rust_root = home.path().join("git/rust-project-scaffold");
        let zig_root = home.path().join("git/zig-project-scaffold");
        seed_test_project_scaffolds(&rust_root, &zig_root);
        let rust_scaffold = EnvVarGuard::set("AUTO_RUNS_RUST_SCAFFOLD_ROOT", &rust_root);
        let zig_scaffold = EnvVarGuard::set("AUTO_RUNS_ZIG_SCAFFOLD_ROOT", &zig_root);
        let home_guard = EnvVarGuard::set("HOME", home.path());
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(workspace.path().join("src/lib.rs"), "pub fn sample() {}\n").unwrap();
        Self {
            _env_lock: env_lock,
            _home: home_guard,
            _home_dir: home,
            _rust_scaffold: rust_scaffold,
            _zig_scaffold: zig_scaffold,
            workspace,
        }
    }

    fn make_run(&self, workflow_mode: AutonomousWorkflowMode) -> CoordinationRun {
        CoordinationRun {
            run_id: Uuid::new_v4().to_string(),
            root_chat_id: Uuid::new_v4().to_string(),
            workspace_path: self.workspace.path().display().to_string(),
            execution_environment: AutonomousExecutionEnvironment::Host,
            runsc_workspace_includes: Vec::new(),
            provider: "codex".to_string(),
            goal: "exercise coordination".to_string(),
            workflow_mode,
            design_doc_path: "docs/design.md".to_string(),
            completion_criteria: "Tests capture the requested transitions.".to_string(),
            max_iterations: 2,
            status: CoordinationRunStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            created_by_instance: None,
            max_agents: 2,
            allow_write_tasks: true,
            require_review_before_apply: false,
            auto_retry_failed_tasks: true,
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

    fn make_task(
        &self,
        run: &CoordinationRun,
        task_id: &str,
        kind: CoordinationTaskKind,
        assigned_provider: Option<&str>,
        title: &str,
        read_scope: &[&str],
        write_scope: &[&str],
        dependencies: &[&str],
        status: CoordinationTaskStatus,
    ) -> CoordinationTask {
        self.make_task_with_options(
            run,
            task_id,
            None,
            kind,
            assigned_provider,
            title,
            vec!["filesystem".to_string(), "search".to_string()],
            read_scope
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            write_scope
                .iter()
                .map(|scope| (*scope).to_string())
                .collect(),
            dependencies.iter().map(|dep| (*dep).to_string()).collect(),
            status,
        )
    }

    fn make_task_with_options(
        &self,
        run: &CoordinationRun,
        task_id: &str,
        parent_task_id: Option<&str>,
        kind: CoordinationTaskKind,
        assigned_provider: Option<&str>,
        title: &str,
        allowed_tools: Vec<String>,
        read_scope: Vec<String>,
        write_scope: Vec<String>,
        dependencies: Vec<String>,
        status: CoordinationTaskStatus,
    ) -> CoordinationTask {
        CoordinationTask {
            task_id: task_id.to_string(),
            run_id: run.run_id.clone(),
            parent_task_id: parent_task_id.map(str::to_string),
            kind,
            assigned_provider: assigned_provider.map(str::to_string),
            task_category: None,
            provider_hint: None,
            required_role: None,
            iteration: None,
            title: title.to_string(),
            prompt: format!("Prompt for {title}"),
            cwd: run.workspace_path.clone(),
            allowed_tools,
            read_scope,
            write_scope,
            acceptance_checks: vec!["check one".to_string(), "check two".to_string()],
            review_metadata: None,
            dependencies,
            execution_generation: default_execution_generation(),
            dependency_generation: None,
            attempt_count: 0,
            max_attempts: 3,
            publish_attempt_count: 0,
            estimated_minutes: Some(default_task_estimated_minutes_for_kind(&kind)),
            status,
            claimed_by: None,
            artifact_summary: None,
            failure_kind: None,
            recovery_action: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn persist_run_with_tasks(&self, run: &CoordinationRun, tasks: &[CoordinationTask]) {
        let mut persisted_run = run.clone();
        persisted_run.task_ids = tasks.iter().map(|task| task.task_id.clone()).collect();
        write_json_atomic(&run_path(&persisted_run.run_id), &persisted_run).unwrap();
        for task in tasks {
            write_json_atomic(&task_path(&persisted_run.run_id, &task.task_id), task).unwrap();
        }
    }

    fn write_artifact(&self, artifact: &TaskArtifact) {
        fs::create_dir_all(artifact_dir(&artifact.run_id, &artifact.task_id)).unwrap();
        write_json_atomic(
            &artifact_result_path(&artifact.run_id, &artifact.task_id),
            artifact,
        )
        .unwrap();
        fs::write(
            artifact_notes_path(&artifact.run_id, &artifact.task_id),
            &artifact.body,
        )
        .unwrap();
    }

    fn write_artifact_materialized_files(
        &self,
        run_id: &str,
        task_id: &str,
        files: &[(&str, &str)],
    ) {
        let payload = files
            .iter()
            .map(|(path, content)| MaterializedArtifactFile {
                path: (*path).to_string(),
                content: (*content).to_string(),
            })
            .collect::<Vec<_>>();
        write_json_atomic(&artifact_materialized_files_path(run_id, task_id), &payload).unwrap();
    }
}

// Self-contained scaffold inputs: runtime copy tests must not depend on an
// operator's private checkout or installed language tooling.
fn seed_test_project_scaffolds(rust: &Path, zig: &Path) {
    let files = [
        ("Cargo.toml", "[workspace]\nmembers = [\"apps/github-branch-fix\", \"crates/scaffold-allocator\"]\nresolver = \"2\"\n[workspace.dependencies]\ncap = \"0.1.2\"\n[profile.release]\nopt-level = 1\n[profile.ship]\ninherits = \"release\"\nlto = true\n[profile.coverage]\ninherits = \"dev\"\n"),
        (".cargo/config.toml", "[target.x86_64-unknown-linux-gnu]\nlinker = \"clang\"\nrustflags = [\"-C\", \"link-arg=-fuse-ld=mold\"]\n[build]\ntarget-dir = \"target/release\"\nrustc-wrapper = \"sccache\"\njobs = 1\n"),
        (".gitignore", "target\n**/*.rs.bk\n"),
        ("rust-toolchain.toml", "[toolchain]\nchannel = \"1.94.1\"\ncomponents = [\"rustfmt\"]\nprofile = \"minimal\"\n"),
        ("rustfmt.toml", "edition = \"2021\"\nstyle_edition = \"2021\"\nmax_width = 100\nnewline_style = \"Unix\"\nuse_small_heuristics = \"Default\"\n"),
        ("apps/github-branch-fix/Cargo.toml", "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("apps/github-branch-fix/src/main.rs", "fn main() {}\n"),
        ("crates/scaffold-allocator/Cargo.toml", "[package]\nname = \"scaffold-allocator\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
        ("crates/scaffold-allocator/src/lib.rs", "pub fn fixture() {}\n"),
        (".githooks/pre-commit", "#!/bin/sh\nexit 0\n"),
    ];
    for (name, contents) in files {
        let path = rust.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    for (name, contents) in [
        ("build.zig", "// test scaffold\n"),
        ("build.zig.zon", ".{}\n"),
        ("lib/compiler.zig", "pub fn fixture() void {}\n"),
    ] {
        let path = zig.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

fn persist_live_test_claim(task: &CoordinationTask) {
    let mut task = task.clone();
    let pid = std::process::id();
    task.claimed_by = Some(pid);
    write_json_atomic(&task_path(&task.run_id, &task.task_id), &task).unwrap();
    index_task(&task);
    assert!(create_claim_file(&TaskClaim {
        run_id: task.run_id.clone(),
        task_id: task.task_id.clone(),
        provider: task
            .assigned_provider
            .clone()
            .unwrap_or_else(|| "codex".to_string()),
        instance_pid: pid,
        launcher_pid: Some(pid),
        lifecycle_phase: ClaimLifecyclePhase::Claimed,
        worker_pid: None,
        claimed_at: Utc::now(),
        worker_spawned_at: None,
        worker_adopted_at: None,
        execution_started_at: None,
        worker_exited_at: None,
        worker_exit_detail: None,
        last_heartbeat_at: Utc::now(),
    })
    .unwrap());
}
