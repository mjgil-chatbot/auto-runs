use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rust_debug::tracing::{debug, info, warn};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::model::ModelProvider;
use crate::{AutonomousExecutionEnvironment, TurnPolicy};
mod parsing;
mod parsing_actions;
mod runtime;
mod runtime_support;
#[cfg(test)]
mod tests;
/// Number of turns without progress before issuing a soft nudge to council members.
/// After 3 turns, the council may be stuck in circular discussion.
const SOFT_STALL_TURNS: u32 = 3;
/// Number of turns without progress before issuing a firm nudge with explicit directives.
/// After 6 turns, stronger intervention is needed.
const FIRM_STALL_TURNS: u32 = 6;
/// Number of turns without progress before terminating the council run as failed.
/// After 9 turns, the council is unlikely to make progress autonomously.
const TERMINAL_STALL_TURNS: u32 = 9;
const STALE_CLAIM_TTL_SECS: i64 = 180;
const SURVEY_ARTIFACT_ID: &str = "council-survey";
#[derive(Serialize, Deserialize, Clone, Debug)]
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
impl CouncilEntry {
    fn new(speaker: CouncilSpeaker, kind: CouncilEntryKind, content: impl Into<String>) -> Self {
        Self {
            entry_id: Uuid::new_v4().to_string(),
            at: Utc::now(),
            speaker,
            kind,
            content: content.into(),
            mentions: Vec::new(),
            claims: Vec::new(),
            diffs: Vec::new(),
        }
    }}
impl CouncilEntry {
    pub fn speaker_label(&self) -> &'static str {
        self.speaker.label()
    }}
impl CouncilEntry {
    pub fn kind_label(&self) -> &'static str {
        self.kind.label()
    }}
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
impl CouncilSpeaker {
    const MODEL_ORDER: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Minimax,
        Self::Qwen,
        Self::Gemini,
    ];}
impl CouncilSpeaker {
    pub fn label(&self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Minimax => "MiniMax",
            Self::Qwen => "Qwen",
            Self::Gemini => "Gemini",
            Self::User => "User",
        }
    }}
impl CouncilSpeaker {
    pub fn mention_name(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Minimax => "minimax",
            Self::Qwen => "qwen",
            Self::Gemini => "gemini",
            Self::User => "user",
        }
    }}
impl CouncilSpeaker {
    fn is_model(&self) -> bool {
        matches!(
            self,
            Self::Claude | Self::Codex | Self::Minimax | Self::Qwen | Self::Gemini
        )
    }}
impl CouncilSpeaker {
    fn model_provider(&self) -> Option<ModelProvider> {
        match self {
            Self::Claude => Some(ModelProvider::Claude),
            Self::Codex => Some(ModelProvider::Codex),
            Self::Minimax => Some(ModelProvider::MiniMax),
            Self::Qwen => Some(ModelProvider::Qwen),
            Self::Gemini => Some(ModelProvider::Gemini),
            Self::System | Self::User => None,
        }
    }}
impl CouncilSpeaker {
    fn from_name(name: &str) -> Option<Self> {
        match name
            .trim()
            .trim_start_matches('@')
            .to_ascii_lowercase()
            .as_str()
        {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "minimax" => Some(Self::Minimax),
            "qwen" => Some(Self::Qwen),
            "gemini" => Some(Self::Gemini),
            "system" => Some(Self::System),
            "user" => Some(Self::User),
            _ => None,
        }
    }}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CouncilEntryKind {
    Discussion,
    ClaimRequest,
    ClaimGranted,
    ClaimRejected,
    ClaimRevoked,
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
    HandoffRequested,
    HandoffAccepted,
    UserInputRequested,
}
impl CouncilEntryKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Discussion => "discussion",
            Self::ClaimRequest => "claim_request",
            Self::ClaimGranted => "claim_granted",
            Self::ClaimRejected => "claim_rejected",
            Self::ClaimRevoked => "claim_revoked",
            Self::ReleaseScope => "release_scope",
            Self::WorkReport => "work_report",
            Self::WorkValidated => "work_validated",
            Self::WorkValidationFailed => "work_validation_failed",
            Self::Delegation => "delegation",
            Self::VoteComplete => "vote_complete",
            Self::VotesAccepted => "votes_accepted",
            Self::VotesRejected => "votes_rejected",
            Self::SystemContext => "system_context",
            Self::SystemNudge => "system_nudge",
            Self::UserInterjection => "user_interjection",
            Self::HandoffRequested => "handoff_requested",
            Self::HandoffAccepted => "handoff_accepted",
            Self::UserInputRequested => "user_input_requested",
        }
    }}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WriteClaim {
    pub scope: String,
    pub claimed_by: CouncilSpeaker,
    pub claimed_at: DateTime<Utc>,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaimRevocationReason {
    StaleTimeout,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RevokedClaim {
    pub scope: String,
    pub claimed_by: CouncilSpeaker,
    pub revoked_at: DateTime<Utc>,
    pub reason: ClaimRevocationReason,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DiffSummary {
    pub file: String,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub description: String,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CouncilRun {
    pub run_id: String,
    pub root_chat_id: String,
    pub workspace_path: String,
    pub goal: String,
    pub completion_criteria: String,
    pub status: CouncilRunStatus,
    pub turn_policy: TurnPolicy,
    pub max_turns: u32,
    pub current_turn: u32,
    #[serde(default)]
    pub active_claims: Vec<WriteClaim>,
    #[serde(default)]
    pub revoked_claims: Vec<RevokedClaim>,
    #[serde(default)]
    pub votes_complete: Vec<CouncilSpeaker>,
    #[serde(default)]
    pub turns_since_progress: u32,
    #[serde(default)]
    pub total_work_reports: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub allow_write_tasks: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub termination_policy: CouncilTerminationPolicy,
}
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CouncilRunStatus {
    #[default]
    Active,
    WaitingForUser,
    VerifyingCompletion,
    Completed,
    Failed,
    Canceled,
}
#[derive(Clone, Debug)]
pub(super) struct ParsedWorkReport {
    pub(super) summary: String,
    pub(super) changed_files: Vec<String>,
    pub(super) diffs: Vec<DiffSummary>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StallAction {
    SoftNudge,
    FirmNudge,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum CouncilAction {
    Mention {
        target: CouncilSpeaker,
    },
    Claim {
        scope: String,
    },
    Release {
        scope: String,
    },
    WorkDone {
        changed_files: Vec<String>,
        summary: String,
    },
    VoteComplete {
        justification: String,
    },
    Handoff {
        target: CouncilSpeaker,
        reason: String,
        scopes: Vec<String>,
    },
    RequestUser {
        prompt: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CouncilTerminationPolicy {
    #[serde(default)]
    pub any: Vec<TerminationCondition>,
    #[serde(default)]
    pub all: Vec<TerminationCondition>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminationCondition {
    MaxTurns(u32),
    MaxStallTurns(u32),
    AllModelsVotedComplete,
    BudgetExceeded,
    UserStop,
}
impl Default for CouncilTerminationPolicy {
    fn default() -> Self {
        Self::for_max_turns(40)
    }
}
impl CouncilTerminationPolicy {
    pub fn for_max_turns(max_turns: u32) -> Self {
        Self {
            any: vec![
                TerminationCondition::MaxTurns(max_turns),
                TerminationCondition::MaxStallTurns(TERMINAL_STALL_TURNS),
                TerminationCondition::BudgetExceeded,
                TerminationCondition::UserStop,
            ],
            all: vec![TerminationCondition::AllModelsVotedComplete],
        }
    }}
