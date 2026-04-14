use super::*;
#[allow(unused_imports)]
use crate::AutonomousExecutionEnvironment;
#[allow(unused_imports)]
use crate::AutonomousWorkflowMode;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
#[allow(unused_imports)]
use std::env;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
#[allow(unused_imports)]
use std::os::unix::process::CommandExt;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
#[allow(unused_imports)]
use std::sync::{mpsc, Arc};
#[allow(unused_imports)]
use std::thread;
#[allow(unused_imports)]
use std::time::Duration;
#[allow(unused_imports)]
use uuid::Uuid;
pub fn should_force_skip_git_repo_check(run: &CoordinationRun, task: &CoordinationTask) -> bool {
    matches!(run.workflow_mode, AutonomousWorkflowMode::PerfHillClimber)
        && matches!(
            task.kind,
            CoordinationTaskKind::PerfBaseline
                | CoordinationTaskKind::PerfParse
                | CoordinationTaskKind::PerfBench
                | CoordinationTaskKind::PerfEvaluate
                | CoordinationTaskKind::PerfApply
        )
}
struct RunscCwdGuard {
    original_cwd: PathBuf,
}
impl RunscCwdGuard {
    fn set(new_cwd: &Path) -> Result<Self> {
        let original_cwd = env::current_dir()?;
        env::set_current_dir(new_cwd)?;
        Ok(Self { original_cwd })
    }}
impl Drop for RunscCwdGuard {
    fn drop(&mut self) {
        let _ = env::set_current_dir(&self.original_cwd);
    }
}
fn task_for_provider_execution(task: &CoordinationTask, run: &CoordinationRun) -> CoordinationTask {
    let mut adjusted = task.clone();
    if run.execution_environment == AutonomousExecutionEnvironment::Runsc {
        adjusted.cwd = autonomous_runsc_workspace_root(&run.run_id)
            .to_string_lossy()
            .into_owned();
    }
    adjusted
}
pub(crate) fn append_runsc_ready_event(
    run: &CoordinationRun,
    task: &CoordinationTask,
    provider_name: &str,
) -> Result<()> {
    if run.execution_environment != AutonomousExecutionEnvironment::Runsc {
        return Ok(());
    }

    let includes = effective_runsc_workspace_includes(run, Some(task));
    let include_summary = if includes.is_empty() {
        "Explicit includes: none".to_string()
    } else {
        format!("Explicit includes: {}", includes.join(", "))
    };
    append_event_with_provider(
        &run.run_id,
        "task_runsc_ready",
        Some(&task.task_id),
        Some(provider_name),
        task.task_category,
        task.provider_hint.as_deref(),
        &format!(
            "Runsc started correctly for {} via {}. Shared workspace root: {}. {}.",
            task.title,
            provider_name,
            autonomous_runsc_workspace_root(&run.run_id).display(),
            include_summary
        ),
    )
}
