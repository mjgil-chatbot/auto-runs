use super::*;
use std::ffi::OsString;
pub const AUTONOMOUS_RUNSC_SHARED_ROOT_ENV: &str = "RUST_CHATBOT_AUTONOMOUS_RUNSC_SHARED_ROOT";
pub const AUTONOMOUS_RUNSC_DISABLE_SYNC_BACK_ENV: &str =
    "RUST_CHATBOT_AUTONOMOUS_RUNSC_DISABLE_SYNC_BACK";
pub struct AutonomousRunscEnvGuard {
    previous_shared_root: Option<OsString>,
    previous_disable_sync_back: Option<OsString>,
    previous_execution_env: Option<OsString>,
    previous_workspace_includes: Option<OsString>,
}
impl AutonomousRunscEnvGuard {
    pub fn activate(run: &CoordinationRun) -> Result<Option<Self>> {
        if run.execution_environment != AutonomousExecutionEnvironment::Runsc {
            return Ok(None);
        }

        let shared_root = ensure_autonomous_runsc_workspace(run)?;
        let previous_shared_root = std::env::var_os(AUTONOMOUS_RUNSC_SHARED_ROOT_ENV);
        let previous_disable_sync_back = std::env::var_os(AUTONOMOUS_RUNSC_DISABLE_SYNC_BACK_ENV);
        let previous_execution_env = std::env::var_os("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV");
        let previous_workspace_includes =
            std::env::var_os("RUST_CHATBOT_AUTONOMOUS_RUNSC_WORKSPACE_INCLUDES");

        std::env::set_var(AUTONOMOUS_RUNSC_SHARED_ROOT_ENV, &shared_root);
        std::env::set_var(AUTONOMOUS_RUNSC_DISABLE_SYNC_BACK_ENV, "1");
        std::env::set_var("RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV", "runsc");
        std::env::set_var(
            "RUST_CHATBOT_AUTONOMOUS_RUNSC_WORKSPACE_INCLUDES",
            serde_json::to_string(&effective_runsc_workspace_includes(run, None))
                .unwrap_or_else(|_| "[]".to_string()),
        );

        Ok(Some(Self {
            previous_shared_root,
            previous_disable_sync_back,
            previous_execution_env,
            previous_workspace_includes,
        }))
    }}
pub fn autonomous_runsc_shared_root(run_id: &str) -> PathBuf {
    run_dir(run_id).join("runsc-shared")
}
pub fn autonomous_runsc_workspace_root(run_id: &str) -> PathBuf {
    autonomous_runsc_shared_root(run_id).join("workspace")
}
pub fn ensure_autonomous_runsc_workspace(run: &CoordinationRun) -> Result<PathBuf> {
    let shared_root = autonomous_runsc_shared_root(&run.run_id);
    let workspace_root = autonomous_runsc_workspace_root(&run.run_id);
    fs::create_dir_all(&workspace_root)
        .with_context(|| format!("failed to create {}", workspace_root.display()))?;

    let initialized = shared_root.join(".initialized");
    if !initialized.exists() {
        populate_autonomous_runsc_workspace(run, &workspace_root)?;
        fs::write(&initialized, b"initialized\n")
            .with_context(|| format!("failed to write {}", initialized.display()))?;
        return Ok(shared_root);
    }

    for include in effective_runsc_workspace_includes(run, None) {
        copy_autonomous_runsc_include_if_missing(
            Path::new(&run.workspace_path),
            &workspace_root,
            Path::new(&include),
        )?;
    }
    Ok(shared_root)
}
pub fn apply_autonomous_runsc_env_to_command(
    command: &mut std::process::Command,
    run: &CoordinationRun,
) -> Result<()> {
    if run.execution_environment != AutonomousExecutionEnvironment::Runsc {
        return Ok(());
    }
    let shared_root = ensure_autonomous_runsc_workspace(run)?;
    command.env(AUTONOMOUS_RUNSC_SHARED_ROOT_ENV, shared_root);
    command.env(AUTONOMOUS_RUNSC_DISABLE_SYNC_BACK_ENV, "1");
    Ok(())
}
pub fn task_runsc_materialization_root(task: &CoordinationTask) -> PathBuf {
    std::env::var_os(AUTONOMOUS_RUNSC_SHARED_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join("workspace"))
        .unwrap_or_else(|| PathBuf::from(&task.cwd))
}
fn populate_autonomous_runsc_workspace(run: &CoordinationRun, workspace_root: &Path) -> Result<()> {
    let source_root = Path::new(&run.workspace_path);
    for include in effective_runsc_workspace_includes(run, None) {
        copy_autonomous_runsc_include(source_root, workspace_root, Path::new(&include))?;
    }
    Ok(())
}
fn copy_autonomous_runsc_include_if_missing(
    source_root: &Path,
    workspace_root: &Path,
    include: &Path,
) -> Result<()> {
    let target = workspace_root.join(include);
    if target.exists() || fs::symlink_metadata(&target).is_ok() {
        return Ok(());
    }
    copy_autonomous_runsc_include(source_root, workspace_root, include)
}
fn copy_autonomous_runsc_include(
    source_root: &Path,
    workspace_root: &Path,
    include: &Path,
) -> Result<()> {
    let source = source_root.join(include);
    if !source.exists() && fs::symlink_metadata(&source).is_err() {
        return Ok(());
    }
    let target = workspace_root.join(include);
    copy_path(source, target)
}
fn copy_path(source: PathBuf, destination: PathBuf) -> Result<()> {
    let metadata = fs::symlink_metadata(&source)
        .with_context(|| format!("failed to stat {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        copy_symlink(&source, &destination)?;
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(&destination)
            .with_context(|| format!("failed to create {}", destination.display()))?;
        for entry in fs::read_dir(&source)
            .with_context(|| format!("failed to read directory {}", source.display()))?
        {
            let entry = entry?;
            copy_path(entry.path(), destination.join(entry.file_name()))?;
        }
        return Ok(());
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(&source, &destination).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    fs::set_permissions(&destination, metadata.permissions())
        .with_context(|| format!("failed to set permissions on {}", destination.display()))?;
    Ok(())
}
fn copy_symlink(source: &Path, destination: &Path) -> Result<()> {
    let target = fs::read_link(source)
        .with_context(|| format!("failed to read symlink {}", source.display()))?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if destination.exists() || fs::symlink_metadata(destination).is_ok() {
        let _ = fs::remove_file(destination);
        let _ = fs::remove_dir_all(destination);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&target, destination).with_context(|| {
            format!(
                "failed to create symlink {} -> {}",
                destination.display(),
                target.display()
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        anyhow::bail!("runsc shared workspace symlink staging is only supported on unix");
    }
    Ok(())
}
