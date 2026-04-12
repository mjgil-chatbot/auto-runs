use super::runtime_support::select_next_speaker;
use super::*;
use crate::coordination;
use tempfile::TempDir;
/// Guard that sets HOME for the test and restores it on drop.
struct HomeGuard(Option<std::ffi::OsString>);
impl HomeGuard {
    fn set(path: &std::path::Path) -> Self {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self(prev)
    }}
impl Drop for HomeGuard {
    fn drop(&mut self) {
        match self.0.as_ref() {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
}
fn sample_council(workspace: &TempDir, turn_policy: TurnPolicy) -> CouncilRun {
    CouncilRun {
        run_id: format!("council-test-{}", Uuid::new_v4().simple()),
        root_chat_id: "chat".to_string(),
        workspace_path: workspace.path().display().to_string(),
        goal: "ship the council flow".to_string(),
        completion_criteria: "validated work is present".to_string(),
        status: CouncilRunStatus::Active,
        turn_policy,
        max_turns: 10,
        current_turn: 0,
        active_claims: Vec::new(),
        revoked_claims: Vec::new(),
        votes_complete: Vec::new(),
        turns_since_progress: 0,
        total_work_reports: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        allow_write_tasks: true,
        notes: String::new(),
        termination_policy: CouncilTerminationPolicy::for_max_turns(10),
    }
}
fn sample_claim(scope: &str, claimed_by: CouncilSpeaker) -> WriteClaim {
    WriteClaim {
        scope: scope.to_string(),
        claimed_by,
        claimed_at: Utc::now(),
    }
}
fn sample_revoked_claim(scope: &str, claimed_by: CouncilSpeaker) -> RevokedClaim {
    RevokedClaim {
        scope: scope.to_string(),
        claimed_by,
        revoked_at: Utc::now(),
        reason: ClaimRevocationReason::StaleTimeout,
    }
}
fn run_dir(run_id: &str) -> std::path::PathBuf {
    coordination::council_json_path(run_id)
        .parent()
        .expect("council run dir")
        .to_path_buf()
}
#[test]
fn mention_driven_turn_prefers_recent_mention() {
    let mut entry = CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::Discussion,
        "@codex please wire the route",
    );
    entry.mentions = vec!["codex".to_string()];
    let ledger = vec![entry];
    assert_eq!(
        select_next_speaker(
            &CouncilRun {
                run_id: "r".to_string(),
                root_chat_id: "c".to_string(),
                workspace_path: ".".to_string(),
                goal: "goal".to_string(),
                completion_criteria: "done".to_string(),
                status: CouncilRunStatus::Active,
                turn_policy: TurnPolicy::MentionDriven,
                max_turns: 10,
                current_turn: 0,
                active_claims: Vec::new(),
                revoked_claims: Vec::new(),
                votes_complete: Vec::new(),
                turns_since_progress: 0,
                total_work_reports: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                allow_write_tasks: true,
                notes: String::new(),
                termination_policy: CouncilTerminationPolicy::for_max_turns(10),
            },
            &ledger
        ),
        Some(CouncilSpeaker::Codex)
    );
}
