use super::*;
#[allow(unused_imports)]
use anyhow::{Context, Result};
#[allow(unused_imports)]
use chrono::{DateTime, Utc};
#[allow(unused_imports)]
use rust_debug::tracing::{debug, warn};
#[allow(unused_imports)]
use schemars::JsonSchema;
#[allow(unused_imports)]
use serde::{Deserialize, Serialize};
#[allow(unused_imports)]
use std::collections::{BTreeMap, BTreeSet};
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
pub const DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES: u32 = 15;
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationRunStatus {
    Planning,
    #[default]
    Active,
    Recovering,
    WaitingForArtifacts,
    WaitingForPolicy,
    Completed,
    Failed,
    Canceled,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationTaskKind {
    #[default]
    SurveyWorkspace,
    PlanCodeReview,
    DraftDesignDoc,
    ConvergeDesignDoc,
    MaterializeDesignDoc,
    PlanFlatTasks,
    RalphPlan,
    GeneratedReadTask,
    GeneratedWriteTask,
    ReviewArtifacts,
    ApplyApprovedEdits,
    RalphExecute,
    RalphVerify,
    PerfBaseline,
    PerfParse,
    PerfApply,
    PerfBench,
    PerfEvaluate,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationTaskOutcome {
    CompletedWithChanges,
    #[default]
    CompletedNoChange,
    Blocked,
    Partial,
    Failed,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskFailureKind {
    #[default]
    Unknown,
    Acceptance,
    ProviderUnavailable,
    ProviderTimeout,
    ClaimLost,
    MissingArtifact,
    Publish,
    BlockedWrite,
    DependencyDrift,
    WorkspaceMutation,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskRecoveryAction {
    #[default]
    None,
    RetryTask,
    RetryPublish,
    ReconcileState,
    ManualIntervention,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TaskExecutionReport {
    #[serde(default)]
    pub outcome: CoordinationTaskOutcome,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub tests_run: Vec<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub remaining_gaps: Vec<String>,
    #[serde(default)]
    pub design_requirements_satisfied: Vec<String>,
    #[serde(default)]
    pub validation_errors: Vec<String>,
    #[serde(default)]
    pub fix_tasks: Vec<GeneratedTaskSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<TaskFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<TaskRecoveryAction>,
}
#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct GeneratedTaskSpec {
    pub title: String,
    #[serde(default)]
    pub provider: Option<String>,
    pub prompt: String,
    pub kind: String,
    #[serde(default = "default_generated_task_estimated_minutes")]
    pub estimated_minutes: u32,
    #[serde(default)]
    pub depends_on_titles: Vec<String>,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub acceptance_checks: Vec<String>,
}
pub const fn default_generated_task_estimated_minutes() -> u32 {
    DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES
}
