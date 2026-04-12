use super::*;
#[allow(unused_imports)]
use crate::{
    AutonomousExecutionEnvironment, AutonomousWorkflowMode, ReviewFixMode, ReviewScope, TurnPolicy,
};
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
/// Worker heartbeat interval in seconds.
pub const TASK_HEARTBEAT_SECS: u64 = 60;
/// Default maximum number of attempts before a task is considered permanently failed.
pub const DEFAULT_TASK_MAX_ATTEMPTS: u32 = 3;
pub const DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES: u32 = 15;
pub fn effective_runsc_workspace_includes(
    run: &CoordinationRun,
    _task: Option<&CoordinationTask>,
) -> Vec<String> {
    let mut includes = Vec::new();
    let mut seen = BTreeSet::new();
    for include in &run.runsc_workspace_includes {
        let trimmed = include.trim();
        if trimmed.is_empty() || !seen.insert(trimmed.to_string()) {
            continue;
        }
        includes.push(trimmed.to_string());
    }
    includes
}
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
pub enum CoordinationTaskStatus {
    #[default]
    Queued,
    Claiming,
    Running,
    ArtifactReady,
    Blocked,
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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    #[default]
    Survey,
    Design,
    Plan,
    DeepWrite,
    QuickWrite,
    Review,
    Verify,
    DocWrite,
    Research,
}
pub use auto_runs_contracts::AgentRole;
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
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ProviderCooldown {
    pub unavailable_until: DateTime<Utc>,
    #[serde(default)]
    pub reason: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoordinationRun {
    pub run_id: String,
    pub root_chat_id: String,
    pub workspace_path: String,
    #[serde(default)]
    pub execution_environment: AutonomousExecutionEnvironment,
    #[serde(default)]
    pub runsc_workspace_includes: Vec<String>,
    pub provider: String,
    pub goal: String,
    #[serde(default)]
    pub workflow_mode: AutonomousWorkflowMode,
    #[serde(default = "default_design_doc_path")]
    pub design_doc_path: String,
    #[serde(default)]
    pub completion_criteria: String,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    pub status: CoordinationRunStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by_instance: Option<u32>,
    pub max_agents: u32,
    pub allow_write_tasks: bool,
    pub require_review_before_apply: bool,
    pub auto_retry_failed_tasks: bool,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_scope: Option<ReviewScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_dimensions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_fix_mode: Option<ReviewFixMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council_turn_policy: Option<TurnPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council_max_turns: Option<u32>,
    #[serde(default)]
    pub provider_cooldowns: BTreeMap<String, ProviderCooldown>,
    pub task_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub processed_mutation_batch_ids: BTreeSet<String>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoordinationTask {
    pub task_id: String,
    pub run_id: String,
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub kind: CoordinationTaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_category: Option<TaskCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_role: Option<AgentRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iteration: Option<u32>,
    pub title: String,
    pub prompt: String,
    pub cwd: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub read_scope: Vec<String>,
    #[serde(default)]
    pub write_scope: Vec<String>,
    #[serde(default)]
    pub acceptance_checks: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_metadata: Option<code_review::ReviewTaskMetadata>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default = "default_execution_generation")]
    pub execution_generation: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_generation: Option<u32>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default = "default_task_max_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub publish_attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    pub status: CoordinationTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_kind: Option<TaskFailureKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<TaskRecoveryAction>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskClaim {
    pub run_id: String,
    pub task_id: String,
    pub provider: String,
    pub instance_pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_pid: Option<u32>,
    #[serde(default)]
    pub lifecycle_phase: ClaimLifecyclePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_pid: Option<u32>,
    pub claimed_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_spawned_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_adopted_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_exited_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_exit_detail: Option<String>,
    pub last_heartbeat_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CoordinationEvent {
    pub at: DateTime<Utc>,
    pub event_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<TaskCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
    pub message: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TaskArtifact {
    pub run_id: String,
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_category: Option<TaskCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_from: Option<String>,
    pub summary: String,
    pub body: String,
    pub completed_at: DateTime<Utc>,
    #[serde(default)]
    pub report: TaskExecutionReport,
}
#[derive(Clone, Debug)]
pub struct UpsertRunInput {
    pub existing_run_id: Option<String>,
    pub root_chat_id: String,
    pub workspace_path: String,
    pub execution_environment: AutonomousExecutionEnvironment,
    pub runsc_workspace_includes: Vec<String>,
    pub provider: String,
    pub goal: String,
    pub workflow_mode: AutonomousWorkflowMode,
    pub design_doc_path: String,
    pub completion_criteria: String,
    pub max_iterations: u32,
    pub created_by_instance: Option<u32>,
    pub max_agents: u32,
    pub allow_write_tasks: bool,
    pub require_review_before_apply: bool,
    pub auto_retry_failed_tasks: bool,
    pub notes: String,
    pub review_scope: Option<ReviewScope>,
    pub review_dimensions: Option<Vec<String>>,
    pub review_fix_mode: Option<ReviewFixMode>,
    pub council_turn_policy: Option<TurnPolicy>,
    pub council_max_turns: Option<u32>,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClaimLifecyclePhase {
    #[default]
    Claimed,
    WorkerSpawned,
    WorkerAdopted,
    TaskExecutionStarted,
    WorkerExited,
}
pub fn default_design_doc_path() -> String {
    crate::default_design_doc_path_for_workflow(&AutonomousWorkflowMode::default())
}
pub const fn default_max_iterations() -> u32 {
    10
}
pub const fn default_task_max_attempts() -> u32 {
    DEFAULT_TASK_MAX_ATTEMPTS
}
pub const fn default_execution_generation() -> u32 {
    1
}
pub const fn default_generated_task_estimated_minutes() -> u32 {
    DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES
}
pub const fn default_task_estimated_minutes_for_kind(kind: &CoordinationTaskKind) -> u32 {
    match kind {
        CoordinationTaskKind::SurveyWorkspace => 10,
        CoordinationTaskKind::PlanCodeReview | CoordinationTaskKind::PlanFlatTasks => 10,
        CoordinationTaskKind::DraftDesignDoc
        | CoordinationTaskKind::ConvergeDesignDoc
        | CoordinationTaskKind::MaterializeDesignDoc => 15,
        CoordinationTaskKind::RalphPlan => 10,
        CoordinationTaskKind::GeneratedReadTask
        | CoordinationTaskKind::RalphVerify
        | CoordinationTaskKind::PerfParse
        | CoordinationTaskKind::PerfEvaluate => 10,
        CoordinationTaskKind::GeneratedWriteTask
        | CoordinationTaskKind::ReviewArtifacts
        | CoordinationTaskKind::ApplyApprovedEdits
        | CoordinationTaskKind::RalphExecute => 15,
        CoordinationTaskKind::PerfBaseline
        | CoordinationTaskKind::PerfApply
        | CoordinationTaskKind::PerfBench => 20,
    }
}
