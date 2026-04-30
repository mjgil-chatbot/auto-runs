use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rust_debug::tracing::{debug, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::config;
use crate::data::ChatSession;
use crate::model::ModelProvider;
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
#[path = "../code_review.rs"]
mod code_review;
pub mod council;
#[cfg(test)]
mod tests;
#[path = "parts/types.rs"]
pub mod types;
pub use types::*;
#[path = "parts/claim_diagnostics.rs"]
pub mod claim_diagnostics;
pub use claim_diagnostics::*;
#[path = "parts/claim_consistency.rs"]
pub mod claim_consistency;
pub use claim_consistency::*;
#[path = "parts/reconciler.rs"]
pub mod reconciler;
#[path = "parts/run_lifecycle.rs"]
pub mod run_lifecycle;
pub use run_lifecycle::*;
#[path = "parts/claims.rs"]
pub mod claims;
pub use claims::*;
#[path = "parts/scopes.rs"]
pub mod scopes;
pub use scopes::*;
#[path = "parts/claims_launch.rs"]
pub mod claims_launch;
pub use claims_launch::*;
#[path = "parts/provider_execution.rs"]
pub mod provider_execution;
pub use provider_execution::*;
#[path = "parts/provider_execution_helpers.rs"]
pub mod provider_execution_helpers;
pub use provider_execution_helpers::*;
#[path = "parts/workspace_activity.rs"]
pub mod workspace_activity;
pub use workspace_activity::*;
#[path = "parts/provider_contracts.rs"]
pub mod provider_contracts;
pub use provider_contracts::*;
#[path = "parts/artifacts.rs"]
pub mod artifacts;
pub use artifacts::*;
#[path = "parts/artifacts_lightweight.rs"]
pub mod artifacts_lightweight;
pub use artifacts_lightweight::*;
#[path = "parts/artifact_helpers.rs"]
pub mod artifact_helpers;
pub use artifact_helpers::*;
#[path = "parts/workspace_ready.rs"]
pub mod workspace_ready;
pub use workspace_ready::*;
#[path = "parts/lightweight_publish.rs"]
pub mod lightweight_publish;
pub use lightweight_publish::*;
#[path = "parts/plan_dependencies.rs"]
pub mod plan_dependencies;
pub use plan_dependencies::*;
#[path = "parts/stub_markers.rs"]
pub mod stub_markers;
pub use stub_markers::*;
#[path = "parts/builtins.rs"]
pub mod builtins;
pub use builtins::*;
#[path = "parts/seeding.rs"]
pub mod seeding;
pub use seeding::*;
#[path = "parts/ralph.rs"]
pub mod ralph;
pub use ralph::*;
#[path = "parts/storage.rs"]
pub mod storage_parts;
pub use storage_parts::*;
#[path = "parts/runsc_shared_workspace.rs"]
pub mod runsc_shared_workspace;
pub use runsc_shared_workspace::*;
#[path = "parts/expansion.rs"]
pub mod expansion;
pub use expansion::*;
#[path = "parts/generated_tasks.rs"]
pub mod generated_tasks;
pub use generated_tasks::*;
#[path = "parts/task_contracts.rs"]
pub mod task_contracts;
pub use task_contracts::*;
#[path = "parts/contract_reasks.rs"]
pub mod contract_reasks;
pub use contract_reasks::*;
#[path = "parts/structured_generation.rs"]
pub mod structured_generation;
#[path = "parts/interrupts.rs"]
pub mod interrupts;
pub use interrupts::*;
#[path = "parts/rails.rs"]
pub mod rails;
pub use rails::*;
#[path = "parts/expansion_support.rs"]
pub mod expansion_support;
pub use expansion_support::*;
#[path = "parts/perf_hill_climber.rs"]
pub mod perf_hill_climber;
pub use perf_hill_climber::*;
#[path = "parts/perf_prompts.rs"]
pub mod perf_prompts;
pub use perf_prompts::*;
#[path = "parts/perf_benchmark.rs"]
pub mod perf_benchmark;
pub use perf_benchmark::*;
#[path = "parts/perf_failure_recovery.rs"]
pub mod perf_failure_recovery;
pub use perf_failure_recovery::*;
#[path = "parts/perf_worktree.rs"]
pub mod perf_worktree;
pub use perf_worktree::*;
#[path = "parts/github_branch_fix.rs"]
pub mod github_branch_fix;
pub use github_branch_fix::*;
#[path = "parts/github_branch_fix_wait.rs"]
pub mod github_branch_fix_wait;
pub use github_branch_fix_wait::*;
#[path = "parts/lightweight_impl.rs"]
pub mod lightweight_impl;
#[path = "parts/ipc.rs"]
pub mod ipc;
pub use ipc::*;
pub use lightweight_impl::*;
