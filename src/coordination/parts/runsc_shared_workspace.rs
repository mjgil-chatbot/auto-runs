use super::*;
use std::ffi::OsString;
pub const AUTONOMOUS_RUNSC_SHARED_ROOT_ENV: &str = "RUST_CHATBOT_AUTONOMOUS_RUNSC_SHARED_ROOT";
pub fn task_runsc_materialization_root(task: &CoordinationTask) -> PathBuf {
    std::env::var_os(AUTONOMOUS_RUNSC_SHARED_ROOT_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|root| root.join("workspace"))
        .unwrap_or_else(|| PathBuf::from(&task.cwd))
}
