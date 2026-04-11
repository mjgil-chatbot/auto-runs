use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DEFAULT_TASK_MAX_ATTEMPTS: u32 = 3;
const DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES: u32 = 15;

#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousWorkflowMode {
    #[default]
    SingleProvider,
    DesignConsensus,
    DesignImplementation,
    LightweightImplementation,
    GithubBranchFix,
    RalphLoop,
    FreeformCouncil,
    CodeReview,
    PerfHillClimber,
}

impl<'de> Deserialize<'de> for AutonomousWorkflowMode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "single_provider" => Ok(Self::SingleProvider),
            "design_consensus" => Ok(Self::DesignConsensus),
            "design_implementation" => Ok(Self::DesignImplementation),
            "lightweight_implementation" => Ok(Self::LightweightImplementation),
            "github_branch_fix" => Ok(Self::GithubBranchFix),
            "ralph_loop" => Ok(Self::RalphLoop),
            "freeform_council" => Ok(Self::FreeformCouncil),
            "code_review" => Ok(Self::CodeReview),
            "perf_hill_climber" => Ok(Self::PerfHillClimber),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "single_provider",
                    "design_consensus",
                    "design_implementation",
                    "lightweight_implementation",
                    "github_branch_fix",
                    "ralph_loop",
                    "freeform_council",
                    "code_review",
                    "perf_hill_climber",
                ],
            )),
        }
    }
}

impl AutonomousWorkflowMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SingleProvider => "single_provider",
            Self::DesignConsensus => "design_consensus",
            Self::DesignImplementation => "design_implementation",
            Self::LightweightImplementation => "lightweight_implementation",
            Self::GithubBranchFix => "github_branch_fix",
            Self::RalphLoop => "ralph_loop",
            Self::FreeformCouncil => "freeform_council",
            Self::CodeReview => "code_review",
            Self::PerfHillClimber => "perf_hill_climber",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::SingleProvider => "Single Provider",
            Self::DesignConsensus => "Design Consensus",
            Self::DesignImplementation => "Design Implementation",
            Self::LightweightImplementation => "Lightweight Implementation",
            Self::GithubBranchFix => "GitHub Branch Fix",
            Self::RalphLoop => "Ralph Loop",
            Self::FreeformCouncil => "Freeform Council",
            Self::CodeReview => "Code Review",
            Self::PerfHillClimber => "Perf Hill Climber",
        }
    }

    pub fn all_variants() -> &'static [Self] {
        &[
            Self::SingleProvider,
            Self::DesignConsensus,
            Self::DesignImplementation,
            Self::LightweightImplementation,
            Self::GithubBranchFix,
            Self::RalphLoop,
            Self::FreeformCouncil,
            Self::CodeReview,
            Self::PerfHillClimber,
        ]
    }
}

fn workflow_doc_slug(workflow_mode: &AutonomousWorkflowMode) -> String {
    workflow_mode.as_str().replace('_', "-")
}

fn short_run_id_hash(run_id: &str) -> String {
    let mut hash: u32 = 0x811c9dc5;
    for byte in run_id.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    format!("{hash:08x}")
}

pub fn default_design_doc_path_for_workflow(workflow_mode: &AutonomousWorkflowMode) -> String {
    format!(
        "docs/autonomous/{}-doc.md",
        workflow_doc_slug(workflow_mode)
    )
}

pub fn default_design_doc_path_for_run(
    workflow_mode: &AutonomousWorkflowMode,
    run_id: &str,
) -> String {
    format!(
        "docs/autonomous/{}-doc-{}.md",
        workflow_doc_slug(workflow_mode),
        short_run_id_hash(run_id)
    )
}

pub fn is_generated_design_doc_path(path: &str) -> bool {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "docs/autonomous/design-doc.md" {
        return true;
    }

    for workflow_mode in AutonomousWorkflowMode::all_variants() {
        let preview_path = default_design_doc_path_for_workflow(workflow_mode);
        if trimmed == preview_path {
            return true;
        }

        let prefix = preview_path.trim_end_matches(".md");
        if let Some(hash) = trimmed
            .strip_prefix(prefix)
            .and_then(|suffix| suffix.strip_prefix('-'))
            .and_then(|suffix| suffix.strip_suffix(".md"))
        {
            if hash.len() == 8 && hash.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return true;
            }
        }
    }

    false
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutonomousExecutionEnvironment {
    #[default]
    Host,
    Runsc,
}

impl AutonomousExecutionEnvironment {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Runsc => "runsc",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TurnPolicy {
    #[default]
    RoundRobin,
    MentionDriven,
    Adaptive,
}

impl TurnPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RoundRobin => "round_robin",
            Self::MentionDriven => "mention_driven",
            Self::Adaptive => "adaptive",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::RoundRobin => "Round Robin",
            Self::MentionDriven => "Mention Driven",
            Self::Adaptive => "Adaptive",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewScope {
    #[default]
    Worktree,
    Diff {
        base: String,
    },
    Paths(Vec<String>),
    Globs(Vec<String>),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReviewFixMode {
    #[default]
    ReportOnly,
    GenerateFixes,
}

impl ReviewFixMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReportOnly => "report_only",
            Self::GenerateFixes => "generate_fixes",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    #[default]
    Orchestrator,
    #[serde(alias = "worker")]
    DeepWorker,
    Reviewer,
    Explorer,
}

impl AgentRole {
    #[allow(non_upper_case_globals)]
    pub const Worker: Self = Self::DeepWorker;
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationRunStatus {
    Planning,
    #[default]
    Active,
    WaitingForArtifacts,
    WaitingForPolicy,
    Completed,
    Failed,
    Canceled,
}

impl CoordinationRunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Active => "active",
            Self::WaitingForArtifacts => "waiting_for_artifacts",
            Self::WaitingForPolicy => "waiting_for_policy",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoordinationTaskOutcome {
    CompletedWithChanges,
    #[default]
    CompletedNoChange,
    Blocked,
    Partial,
    Failed,
}

impl CoordinationTaskOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CompletedWithChanges => "completed_with_changes",
            Self::CompletedNoChange => "completed_no_change",
            Self::Blocked => "blocked",
            Self::Partial => "partial",
            Self::Failed => "failed",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewChunkTarget {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AutomatedToolCheck {
    pub command: String,
    pub status: String,
    #[serde(default)]
    pub summary: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewTaskMetadata {
    pub scope_description: String,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub chunk: Vec<ReviewChunkTarget>,
    #[serde(default)]
    pub automated_checks: Vec<AutomatedToolCheck>,
    #[serde(default)]
    pub profile_path: String,
    #[serde(default)]
    pub profile_warnings: Vec<String>,
    #[serde(default)]
    pub fix_mode: Option<ReviewFixMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ProviderCooldown {
    pub unavailable_until: DateTime<Utc>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
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
    pub review_metadata: Option<ReviewTaskMetadata>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub attempt_count: u32,
    #[serde(default = "default_task_max_attempts")]
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_minutes: Option<u32>,
    pub status: CoordinationTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_by: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_summary: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TaskClaim {
    pub run_id: String,
    pub task_id: String,
    pub provider: String,
    pub instance_pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launcher_pid: Option<u32>,
    pub claimed_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CoordinationRunSnapshot {
    pub run: CoordinationRun,
    pub tasks: Vec<CoordinationTask>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CouncilEntry {
    pub entry_id: String,
    pub at: DateTime<Utc>,
    pub speaker: CouncilSpeaker,
    pub kind: CouncilEntryKind,
    pub content: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub claims: Vec<WriteClaim>,
    #[serde(default)]
    pub diffs: Vec<DiffSummary>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CouncilSpeaker {
    System,
    Claude,
    Codex,
    Minimax,
    Qwen,
    Gemini,
    User,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CouncilEntryKind {
    Discussion,
    ClaimRequest,
    ClaimGranted,
    ClaimRejected,
    ReleaseScope,
    WorkReport,
    WorkValidated,
    WorkValidationFailed,
    Delegation,
    VoteComplete,
    VotesAccepted,
    VotesRejected,
    SystemContext,
    SystemNudge,
    UserInterjection,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct WriteClaim {
    pub scope: String,
    pub claimed_by: CouncilSpeaker,
    pub claimed_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct DiffSummary {
    pub file: String,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub description: String,
}

fn default_design_doc_path() -> String {
    default_design_doc_path_for_workflow(&AutonomousWorkflowMode::default())
}

const fn default_max_iterations() -> u32 {
    10
}

const fn default_task_max_attempts() -> u32 {
    DEFAULT_TASK_MAX_ATTEMPTS
}

const fn default_generated_task_estimated_minutes() -> u32 {
    DEFAULT_GENERATED_TASK_ESTIMATED_MINUTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_design_doc_path_for_run_uses_workflow_slug_and_hash() {
        // Published FNV-1a 32-bit reference vectors protect the stable format.
        assert_eq!(short_run_id_hash(""), "811c9dc5");
        assert_eq!(short_run_id_hash("hello"), "4f9f2cab");
        let path = default_design_doc_path_for_run(
            &AutonomousWorkflowMode::DesignImplementation,
            "run-123",
        );
        assert_eq!(
            path,
            "docs/autonomous/design-implementation-doc-4afd797b.md"
        );
    }

    #[test]
    fn generated_design_doc_path_detection_accepts_preview_and_hashed_forms() {
        assert!(is_generated_design_doc_path(
            "docs/autonomous/design-doc.md"
        ));
        assert!(is_generated_design_doc_path(
            &default_design_doc_path_for_workflow(&AutonomousWorkflowMode::CodeReview)
        ));
        assert!(is_generated_design_doc_path(
            &default_design_doc_path_for_run(&AutonomousWorkflowMode::RalphLoop, "run-456")
        ));
        assert!(!is_generated_design_doc_path("docs/custom-plan.md"));
    }
}
