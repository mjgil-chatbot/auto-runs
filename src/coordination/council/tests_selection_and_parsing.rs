use super::parsing::{
    contains_stub_markers, extract_claims, extract_mentions, extract_releases,
    extract_revoke_votes, extract_vote_justification, extract_work_report,
    parse_and_validate_council_response, release_claim, revoke_stale_claims,
    validate_claim_request, verify_completion,
};
use super::parsing_actions::strip_action_markers;
use super::runtime_support::{
    apply_stall_action, build_council_prompt, check_stall, evaluate_termination,
    select_next_speaker, CouncilTerminationOutcome,
};
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
    }
}

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

// ---------------------------------------------------------------------------
// Speaker selection: round-robin
// ---------------------------------------------------------------------------

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

#[test]
fn parses_claims_releases_and_vote_markers() {
    let text = "[CLAIM src/lib.rs]\n[RELEASE src/old.rs]\n[VOTE_COMPLETE] looks done";
    assert_eq!(extract_claims(text), vec!["src/lib.rs"]);
    assert_eq!(extract_releases(text), vec!["src/old.rs"]);
    assert_eq!(
        extract_vote_justification(text, "looks done"),
        "looks done".to_string()
    );
}

#[test]
fn strip_action_markers_removes_handoff_and_request_user_markers() {
    let text = "[HANDOFF @gemini verify the result]\n[REQUEST_USER should we keep the migration?]\nplain discussion";
    assert_eq!(strip_action_markers(text), "plain discussion");
}

#[test]
fn extracts_work_report_files_from_common_formats() {
    let report = extract_work_report(
        "[WORK_DONE]\nchanged files: src/lib.rs, src/main.rs\nsummary: implemented broker",
        ".",
        CouncilSpeaker::Codex,
    )
    .unwrap();
    assert_eq!(
        report.changed_files,
        vec!["src/lib.rs".to_string(), "src/main.rs".to_string()]
    );
    assert!(report.summary.contains("implemented broker"));
}

#[test]
fn round_robin_turn_advances_after_last_model_speaker() {
    let workspace = TempDir::new().unwrap();
    let ledger = vec![CouncilEntry::new(
        CouncilSpeaker::Gemini,
        CouncilEntryKind::Discussion,
        "verification finished",
    )];

    assert_eq!(
        select_next_speaker(&sample_council(&workspace, TurnPolicy::RoundRobin), &ledger),
        Some(CouncilSpeaker::Claude)
    );
}

#[test]
fn adaptive_turn_prefers_recent_mentioned_speaker_with_active_claims() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::Adaptive);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Gemini));
    let mut mention = CouncilEntry::new(
        CouncilSpeaker::Minimax,
        CouncilEntryKind::Delegation,
        "@gemini please verify the claimed file",
    );
    mention.mentions = vec!["gemini".to_string()];
    let ledger = vec![
        CouncilEntry::new(
            CouncilSpeaker::Claude,
            CouncilEntryKind::Discussion,
            "outlined the next step",
        ),
        mention,
    ];

    assert_eq!(
        select_next_speaker(&council, &ledger),
        Some(CouncilSpeaker::Gemini)
    );
}

#[test]
fn handoff_target_is_selected_before_pending_user_request() {
    let workspace = TempDir::new().unwrap();
    let mut handoff = CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::HandoffRequested,
        "Handoff to Codex: implement the route",
    );
    handoff.mentions = vec!["codex".to_string()];
    let user_request = CouncilEntry::new(
        CouncilSpeaker::Codex,
        CouncilEntryKind::UserInputRequested,
        "Need a user decision before merge",
    );
    let ledger = vec![handoff, user_request];

    assert_eq!(
        select_next_speaker(&sample_council(&workspace, TurnPolicy::RoundRobin), &ledger),
        Some(CouncilSpeaker::Codex)
    );
}

#[test]
fn parse_and_validate_response_records_handoff_acceptance_for_target_speaker() {
    let _home_dir = TempDir::new().unwrap();
    let _env_lock = crate::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _home = HomeGuard::set(_home_dir.path());
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);

    let mut pending_handoff = CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::HandoffRequested,
        "Handoff to Codex: implement the route",
    );
    pending_handoff.mentions = vec!["codex".to_string()];
    let run_id = council.run_id.clone();
    let ledger_path = coordination::ledger_path(&run_id);
    fs::create_dir_all(ledger_path.parent().expect("ledger parent")).unwrap();
    fs::write(
        &ledger_path,
        format!("{}\n", serde_json::to_string(&pending_handoff).unwrap()),
    )
    .unwrap();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "Taking the handoff now.",
        7,
    )
    .unwrap();

    assert!(entries
        .iter()
        .any(|entry| entry.kind == CouncilEntryKind::HandoffAccepted));
    cleanup_council_run(&run_id);
}

#[test]
fn build_council_prompt_describes_explicit_handoff_and_user_request_actions() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);

    let prompt = build_council_prompt(&council, &[], CouncilSpeaker::Claude);

    assert!(prompt.contains("[HANDOFF @codex reason]"));
    assert!(prompt.contains("[REQUEST_USER one clear question]"));
    assert!(prompt.contains("Termination policy:"));
}

#[test]
fn evaluate_termination_uses_policy_max_turns_and_completion_rules() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.current_turn = 3;
    council.termination_policy = CouncilTerminationPolicy {
        any: vec![TerminationCondition::MaxTurns(3)],
        all: Vec::new(),
    };
    assert_eq!(
        evaluate_termination(&council, coordination::CoordinationRunStatus::Active),
        Some(CouncilTerminationOutcome::Fail("max-turns"))
    );

    council.current_turn = 0;
    council.votes_complete = CouncilSpeaker::MODEL_ORDER.to_vec();
    council.termination_policy = CouncilTerminationPolicy {
        any: Vec::new(),
        all: vec![TerminationCondition::AllModelsVotedComplete],
    };
    assert_eq!(
        evaluate_termination(&council, coordination::CoordinationRunStatus::Active),
        Some(CouncilTerminationOutcome::Complete)
    );
}

#[test]
fn extracts_mentions_and_revoke_votes_for_model_speakers() {
    assert_eq!(
        extract_mentions("@codex loop in @gemini, ignore @user and @codex twice"),
        vec!["codex".to_string(), "gemini".to_string()]
    );
    assert_eq!(
        extract_revoke_votes("[REVOKE_VOTE codex]\n[REVOKE_VOTE user]\n[REVOKE_VOTE gemini]"),
        vec![CouncilSpeaker::Codex, CouncilSpeaker::Gemini]
    );
}

#[test]
fn parse_and_validate_response_records_claim_delegation_and_vote() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "@gemini please verify this next\n[CLAIM src/lib.rs]\n[VOTE_COMPLETE] claim is ready",
        44,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|entry| entry.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::Discussion));
    assert!(kinds.contains(&CouncilEntryKind::Delegation));
    assert!(kinds.contains(&CouncilEntryKind::ClaimRequest));
    assert!(kinds.contains(&CouncilEntryKind::ClaimGranted));
    assert!(kinds.contains(&CouncilEntryKind::VoteComplete));
    // Claims are auto-released when a speaker votes complete, so active_claims
    // is empty after this turn (the claim was granted then immediately released).
    assert!(council.active_claims.is_empty());
    assert_eq!(council.votes_complete, vec![CouncilSpeaker::Codex]);
    // A granted claim resets the stall counter so the claimer has time to come
    // back around the council rotation and report validated work.
    assert_eq!(council.turns_since_progress, 0);
}

#[test]
fn parse_and_validate_response_stores_validated_work_artifact() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/lib.rs"), "pub fn ready() {}\n").unwrap();

    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Codex));
    let run_id = council.run_id.clone();

    let council_run_dir = run_dir(&run_id);
    let _ = fs::remove_dir_all(&council_run_dir);

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[WORK_DONE]\nchanged files: src/lib.rs\nsummary: wired the council artifact path",
        77,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|entry| entry.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::WorkReport));
    assert!(kinds.contains(&CouncilEntryKind::WorkValidated));
    assert_eq!(council.total_work_reports, 1);

    let artifact_dirs = fs::read_dir(council_run_dir.join("artifacts"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(artifact_dirs.len(), 1);

    let raw = fs::read_to_string(artifact_dirs[0].join("result.json")).unwrap();
    let artifact: coordination::TaskArtifact = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        artifact.report.outcome,
        coordination::CoordinationTaskOutcome::CompletedWithChanges
    );
    assert_eq!(
        artifact.report.changed_files,
        vec!["src/lib.rs".to_string()]
    );

    let _ = fs::remove_dir_all(council_run_dir);
}

#[test]
fn parse_and_validate_response_rejects_work_outside_claimed_scope() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/lib.rs"), "pub fn ready() {}\n").unwrap();
    fs::write(workspace.path().join("src/other.rs"), "pub fn other() {}\n").unwrap();

    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Codex));
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[WORK_DONE]\nchanged files: src/other.rs\nsummary: touched a different file",
        91,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|entry| entry.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::WorkReport));
    assert!(kinds.contains(&CouncilEntryKind::WorkValidationFailed));
    assert_eq!(council.total_work_reports, 0);
}

#[test]
fn verify_completion_reports_incomplete_state_and_bad_artifacts() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));

    let council_run_dir = run_dir(&council.run_id);
    let result_path = council_run_dir
        .join("artifacts")
        .join("bad-artifact")
        .join("result.json");
    fs::create_dir_all(result_path.parent().unwrap()).unwrap();
    let artifact = coordination::TaskArtifact {
        run_id: council.run_id.clone(),
        task_id: "bad-artifact".to_string(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: "artifact".to_string(),
        body: "artifact body".to_string(),
        completed_at: Utc::now(),
        report: coordination::TaskExecutionReport {
            outcome: coordination::CoordinationTaskOutcome::Partial,
            summary: "artifact".to_string(),
            details: "artifact body".to_string(),
            validation_errors: vec!["report mismatch".to_string()],
            ..Default::default()
        },
    };
    fs::write(
        &result_path,
        serde_json::to_string_pretty(&artifact).unwrap(),
    )
    .unwrap();

    let issues = verify_completion(&council.run_id, &council).unwrap();
    assert!(issues
        .iter()
        .any(|issue| issue.contains("scope(s) are still claimed")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("no validated work reports")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("still has validation errors")));
    assert!(issues
        .iter()
        .any(|issue| issue.contains("ended with outcome partial")));

    let _ = fs::remove_dir_all(council_run_dir);
}

#[test]
fn verify_completion_accepts_clean_council_state() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.total_work_reports = 1;

    assert!(verify_completion(&council.run_id, &council)
        .unwrap()
        .is_empty());
}

// ---------------------------------------------------------------------------
// Extended extraction tests
// ---------------------------------------------------------------------------

#[test]
fn extract_claims_handles_multiple_claims() {
    let text = "[CLAIM src/lib.rs]\nsome text\n[CLAIM tests/mod.rs]\n[CLAIM Cargo.toml]";
    assert_eq!(
        extract_claims(text),
        vec![
            "src/lib.rs".to_string(),
            "tests/mod.rs".to_string(),
            "Cargo.toml".to_string()
        ]
    );
}

#[test]
fn extract_claims_ignores_malformed_brackets() {
    assert!(extract_claims("[CLAIM ").is_empty());
    assert!(extract_claims("[CLAIM]").is_empty());
    assert!(extract_claims("CLAIM src/lib.rs]").is_empty());
}

#[test]
fn extract_releases_handles_multiple_releases() {
    let text = "[RELEASE src/a.rs]\n[RELEASE src/b.rs]";
    assert_eq!(
        extract_releases(text),
        vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
    );
}

#[test]
fn extract_mentions_ignores_unknown_names() {
    assert!(extract_mentions("@unknown @nobody @random").is_empty());
}

#[test]
fn extract_mentions_handles_empty_input() {
    assert!(extract_mentions("").is_empty());
    assert!(extract_mentions("no mentions here").is_empty());
}

#[test]
fn extract_revoke_votes_ignores_non_model_speakers() {
    let result = extract_revoke_votes("[REVOKE_VOTE system]\n[REVOKE_VOTE user]");
    assert!(result.is_empty());
}

#[test]
fn extract_revoke_votes_handles_empty_input() {
    assert!(extract_revoke_votes("").is_empty());
    assert!(extract_revoke_votes("no revokes").is_empty());
}

// ---------------------------------------------------------------------------
// strip_action_markers
// ---------------------------------------------------------------------------

#[test]
fn strip_action_markers_preserves_discussion_content() {
    let text = "hello world\n[CLAIM src/lib.rs]\nmore discussion\n[RELEASE src/old.rs]\nfinal";
    let cleaned = strip_action_markers(text);
    assert!(cleaned.contains("hello world"));
    assert!(cleaned.contains("more discussion"));
    assert!(cleaned.contains("final"));
    assert!(!cleaned.contains("[CLAIM"));
    assert!(!cleaned.contains("[RELEASE"));
}

#[test]
fn strip_action_markers_removes_standalone_vote_complete() {
    let text = "[VOTE_COMPLETE]\nI think we're done";
    let cleaned = strip_action_markers(text);
    assert!(!cleaned.contains("[VOTE_COMPLETE]"));
    assert!(cleaned.contains("I think we're done"));
}

#[test]
fn strip_action_markers_removes_standalone_work_done() {
    let text = "[WORK_DONE]\nsummary: finished";
    let cleaned = strip_action_markers(text);
    assert!(!cleaned.contains("[WORK_DONE]"));
    assert!(cleaned.contains("summary: finished"));
}

#[test]
fn strip_action_markers_returns_empty_for_markers_only() {
    let text = "[CLAIM src/lib.rs]\n[RELEASE src/old.rs]\n[VOTE_COMPLETE]\n[WORK_DONE]";
    let cleaned = strip_action_markers(text);
    assert!(cleaned.is_empty());
}

// ---------------------------------------------------------------------------
// extract_vote_justification edge cases
// ---------------------------------------------------------------------------

#[test]
fn extract_vote_justification_uses_clean_content_when_no_text_after_marker() {
    let response = "[VOTE_COMPLETE]";
    let justification = extract_vote_justification(response, "I believe the work is done");
    assert_eq!(justification, "I believe the work is done");
}

#[test]
fn extract_vote_justification_uses_default_when_both_empty() {
    let justification = extract_vote_justification("no marker here", "");
    assert_eq!(justification, "Speaker voted complete.");
}

#[test]
fn extract_vote_justification_prefers_inline_text() {
    let response = "[VOTE_COMPLETE] all tests pass and scopes released";
    let justification = extract_vote_justification(response, "fallback");
    assert_eq!(justification, "all tests pass and scopes released");
}

#[test]
fn extract_vote_justification_strips_trailing_claim_on_next_line() {
    let response = "[VOTE_COMPLETE] done\n[CLAIM src/lib.rs]";
    let justification = extract_vote_justification(response, "fallback");
    assert_eq!(justification, "done");
}

#[test]
fn extract_vote_justification_strips_trailing_release_on_next_line() {
    let response = "[VOTE_COMPLETE] all good\n[RELEASE src/old.rs]\n[CLAIM src/new.rs]";
    let justification = extract_vote_justification(response, "fallback");
    assert_eq!(justification, "all good");
}

#[test]
fn extract_vote_justification_strips_inline_claim_on_same_line() {
    let response = "[VOTE_COMPLETE] looks good [CLAIM src/lib.rs]";
    let justification = extract_vote_justification(response, "fallback");
    assert_eq!(justification, "looks good");
}

#[test]
fn extract_vote_justification_falls_back_when_only_markers_after_vote() {
    let response = "[VOTE_COMPLETE]\n[CLAIM src/lib.rs]\n[RELEASE src/old.rs]";
    let justification = extract_vote_justification(response, "real justification");
    assert_eq!(justification, "real justification");
}

#[test]
fn extract_vote_justification_strips_work_done_from_same_line() {
    let response = "[VOTE_COMPLETE] finished [WORK_DONE]";
    let justification = extract_vote_justification(response, "fallback");
    assert_eq!(justification, "finished");
}

// ---------------------------------------------------------------------------
// Work report extraction: various formats
// ---------------------------------------------------------------------------

#[test]
fn extract_work_report_from_inline_changed_markers() {
    let text =
        "[WORK_DONE]\n[CHANGED src/main.rs]\n[CHANGED src/lib.rs]\nsummary: added entry point";
    let report = extract_work_report(text, ".", CouncilSpeaker::Claude).unwrap();
    assert_eq!(report.changed_files.len(), 2);
    assert!(report.changed_files.contains(&"src/main.rs".to_string()));
    assert!(report.changed_files.contains(&"src/lib.rs".to_string()));
}

#[test]
fn extract_work_report_from_diff_summary_lines() {
    let text = "[WORK_DONE]\nsrc/lib.rs | +10 -3\nsrc/main.rs | +5 -0\nsummary: refactored";
    let report = extract_work_report(text, ".", CouncilSpeaker::Codex).unwrap();
    assert_eq!(report.changed_files.len(), 2);
    assert!(report.changed_files.contains(&"src/lib.rs".to_string()));
    assert!(report.changed_files.contains(&"src/main.rs".to_string()));
}

#[test]
fn extract_work_report_from_bullet_list() {
    let text = "[WORK_DONE]\n- src/lib.rs\n- src/main.rs\nsummary: implemented feature";
    let report = extract_work_report(text, ".", CouncilSpeaker::Gemini).unwrap();
    assert_eq!(report.changed_files.len(), 2);
    assert!(report.changed_files.contains(&"src/lib.rs".to_string()));
    assert!(report.changed_files.contains(&"src/main.rs".to_string()));
}

#[test]
fn extract_work_report_returns_none_without_marker() {
    assert!(extract_work_report("no work done marker here", ".", CouncilSpeaker::Claude).is_none());
}

#[test]
fn extract_work_report_deduplicates_files() {
    let text = "[WORK_DONE]\nchanged files: src/lib.rs, src/lib.rs\nsummary: deduplicate test";
    let report = extract_work_report(text, ".", CouncilSpeaker::Codex).unwrap();
    assert_eq!(report.changed_files.len(), 1);
    assert_eq!(report.changed_files[0], "src/lib.rs");
}

#[test]
fn extract_work_report_rejects_traversal_paths() {
    let text = "[WORK_DONE]\nchanged files: ../../../etc/passwd\nsummary: attack";
    let report = extract_work_report(text, ".", CouncilSpeaker::Codex).unwrap();
    assert!(report.changed_files.is_empty());
}

#[test]
fn extract_work_report_generates_default_summary() {
    let text = "[WORK_DONE]\nchanged files: src/lib.rs";
    let report = extract_work_report(text, ".", CouncilSpeaker::Minimax).unwrap();
    assert!(report.summary.contains("MiniMax"));
    assert!(report.summary.contains("reported completed work"));
}

// ---------------------------------------------------------------------------
// Speaker selection: round-robin edge cases
// ---------------------------------------------------------------------------

#[test]
fn round_robin_starts_with_claude_on_empty_ledger() {
    let workspace = TempDir::new().unwrap();
    assert_eq!(
        select_next_speaker(&sample_council(&workspace, TurnPolicy::RoundRobin), &[]),
        Some(CouncilSpeaker::Claude)
    );
}

#[test]
fn round_robin_cycles_through_all_models() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let rotation = [
        CouncilSpeaker::Claude,
        CouncilSpeaker::Codex,
        CouncilSpeaker::Minimax,
        CouncilSpeaker::Qwen,
        CouncilSpeaker::Gemini,
    ];
    for i in 0..rotation.len() {
        let ledger = vec![CouncilEntry::new(
            rotation[i],
            CouncilEntryKind::Discussion,
            "turn",
        )];
        let expected_next = rotation[(i + 1) % rotation.len()];
        assert_eq!(
            select_next_speaker(&council, &ledger),
            Some(expected_next),
            "after {:?} expected {:?}",
            rotation[i],
            expected_next
        );
    }
}

#[test]
fn round_robin_skips_system_entries_for_last_speaker() {
    let workspace = TempDir::new().unwrap();
    let ledger = vec![
        CouncilEntry::new(
            CouncilSpeaker::Claude,
            CouncilEntryKind::Discussion,
            "outline",
        ),
        CouncilEntry::new(
            CouncilSpeaker::System,
            CouncilEntryKind::SystemNudge,
            "system message",
        ),
    ];
    assert_eq!(
        select_next_speaker(&sample_council(&workspace, TurnPolicy::RoundRobin), &ledger),
        Some(CouncilSpeaker::Codex)
    );
}

// ---------------------------------------------------------------------------
// Speaker selection: mention-driven edge cases
// ---------------------------------------------------------------------------
