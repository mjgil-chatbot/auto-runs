#[test]
fn mention_driven_falls_back_to_round_robin_without_mentions() {
    let workspace = TempDir::new().unwrap();
    let ledger = vec![CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::Discussion,
        "no mentions here",
    )];
    assert_eq!(
        select_next_speaker(
            &sample_council(&workspace, TurnPolicy::MentionDriven),
            &ledger
        ),
        Some(CouncilSpeaker::Codex)
    );
}

#[test]
fn mention_driven_skips_last_speaker_when_mentioned() {
    let workspace = TempDir::new().unwrap();
    let mut entry = CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::Discussion,
        "@claude self-mention should be skipped",
    );
    entry.mentions = vec!["claude".to_string()];
    let ledger = vec![entry];
    // Claude is last speaker and only mention, should fall back to round-robin → Codex
    assert_eq!(
        select_next_speaker(
            &sample_council(&workspace, TurnPolicy::MentionDriven),
            &ledger
        ),
        Some(CouncilSpeaker::Codex)
    );
}

// ---------------------------------------------------------------------------
// Speaker selection: adaptive edge cases
// ---------------------------------------------------------------------------

#[test]
fn adaptive_without_mentions_or_claims_avoids_last_speaker() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::Adaptive);
    let ledger = vec![CouncilEntry::new(
        CouncilSpeaker::Claude,
        CouncilEntryKind::Discussion,
        "plain text",
    )];
    let speaker = select_next_speaker(&council, &ledger);
    assert!(speaker.is_some());
    assert_ne!(speaker, Some(CouncilSpeaker::Claude));
}

#[test]
fn adaptive_favors_speaker_with_many_claims() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::Adaptive);
    // Give Codex 3 claims
    council
        .active_claims
        .push(sample_claim("src/a.rs", CouncilSpeaker::Codex));
    council
        .active_claims
        .push(sample_claim("src/b.rs", CouncilSpeaker::Codex));
    council
        .active_claims
        .push(sample_claim("src/c.rs", CouncilSpeaker::Codex));
    let ledger = vec![CouncilEntry::new(
        CouncilSpeaker::Gemini,
        CouncilEntryKind::Discussion,
        "done verifying",
    )];
    // Codex has 3 claims (+9), not last speaker (+2), turns_since bonus
    assert_eq!(
        select_next_speaker(&council, &ledger),
        Some(CouncilSpeaker::Codex)
    );
}

#[test]
fn adaptive_always_returns_some() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::Adaptive);
    assert!(select_next_speaker(&council, &[]).is_some());
}

// ---------------------------------------------------------------------------
// Stall detection state machine
// ---------------------------------------------------------------------------

#[test]
fn check_stall_returns_none_below_threshold() {
    assert!(check_stall(0).is_none());
    assert!(check_stall(1).is_none());
    assert!(check_stall(2).is_none());
}

#[test]
fn check_stall_soft_nudge_at_three() {
    assert_eq!(check_stall(3), Some(StallAction::SoftNudge));
}

#[test]
fn check_stall_firm_nudge_at_six() {
    assert_eq!(check_stall(6), Some(StallAction::FirmNudge));
}

#[test]
fn check_stall_firm_nudge_at_and_above_nine() {
    assert_eq!(check_stall(9), Some(StallAction::FirmNudge));
    assert_eq!(check_stall(10), Some(StallAction::FirmNudge));
    assert_eq!(check_stall(100), Some(StallAction::FirmNudge));
}

#[test]
fn check_stall_soft_nudge_for_four_and_five() {
    assert_eq!(check_stall(4), Some(StallAction::SoftNudge));
    assert_eq!(check_stall(5), Some(StallAction::SoftNudge));
}

#[test]
fn check_stall_firm_nudge_for_seven_and_eight() {
    assert_eq!(check_stall(7), Some(StallAction::FirmNudge));
    assert_eq!(check_stall(8), Some(StallAction::FirmNudge));
}

#[test]
fn revoke_stale_claims_revokes_claims_at_three_minutes() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let claimed_at = Utc::now() - chrono::Duration::seconds(STALE_CLAIM_TTL_SECS);
    council.active_claims.push(WriteClaim {
        scope: "src/lib.rs".to_string(),
        claimed_by: CouncilSpeaker::Gemini,
        claimed_at,
    });

    let entries = revoke_stale_claims(&mut council, Utc::now());

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, CouncilEntryKind::ClaimRevoked);
    assert!(entries[0].content.contains("may not reclaim"));
    assert!(council.active_claims.is_empty());
    assert_eq!(council.revoked_claims.len(), 1);
    assert_eq!(council.revoked_claims[0].scope, "src/lib.rs");
    assert_eq!(council.revoked_claims[0].claimed_by, CouncilSpeaker::Gemini);
}

#[test]
fn revoke_stale_claims_keeps_recent_claims_active() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.active_claims.push(WriteClaim {
        scope: "src/lib.rs".to_string(),
        claimed_by: CouncilSpeaker::Gemini,
        claimed_at: Utc::now() - chrono::Duration::seconds(STALE_CLAIM_TTL_SECS - 1),
    });

    let entries = revoke_stale_claims(&mut council, Utc::now());

    assert!(entries.is_empty());
    assert_eq!(council.active_claims.len(), 1);
    assert!(council.revoked_claims.is_empty());
}

#[test]
fn apply_stall_soft_nudge_produces_system_entry() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.turns_since_progress = 3;
    let run_id = council.run_id.clone();
    let entries = apply_stall_action(&run_id, &mut council, StallAction::SoftNudge);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].speaker, CouncilSpeaker::System);
    assert_eq!(entries[0].kind, CouncilEntryKind::SystemNudge);
    assert!(entries[0].content.contains("3 turns"));
    assert_eq!(council.status, CouncilRunStatus::Active);
}

#[test]
fn apply_stall_firm_nudge_produces_system_entry() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.turns_since_progress = 6;
    let run_id = council.run_id.clone();
    let entries = apply_stall_action(&run_id, &mut council, StallAction::FirmNudge);
    assert_eq!(entries.len(), 1);
    assert!(entries[0].content.contains("6 turn"));
    assert_eq!(council.status, CouncilRunStatus::Active);
}

#[test]
fn evaluate_termination_fails_when_policy_max_stall_limit_is_reached() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.turns_since_progress = 4;
    council.termination_policy = CouncilTerminationPolicy {
        any: vec![TerminationCondition::MaxStallTurns(4)],
        all: Vec::new(),
    };

    assert_eq!(
        evaluate_termination(&council, coordination::CoordinationRunStatus::Active),
        Some(CouncilTerminationOutcome::Fail("max-stall-turns"))
    );
}

// ---------------------------------------------------------------------------
// Vote state transitions
// ---------------------------------------------------------------------------

#[test]
fn parse_validate_increments_turns_since_progress_on_no_action() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run_id = council.run_id.clone();
    council.turns_since_progress = 2;

    let _ = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "just chatting, no markers",
        1,
    )
    .unwrap();

    assert_eq!(council.turns_since_progress, 3);
}

#[test]
fn parse_validate_resets_turns_since_progress_on_validated_work() {
    let workspace = TempDir::new().unwrap();
    fs::create_dir_all(workspace.path().join("src")).unwrap();
    fs::write(workspace.path().join("src/lib.rs"), "pub fn ready() {}\n").unwrap();

    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Codex));
    let run_id = council.run_id.clone();
    council.turns_since_progress = 5;

    let _ = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[WORK_DONE]\nchanged files: src/lib.rs\nsummary: implemented feature",
        2,
    )
    .unwrap();

    // Only validated work resets the stall counter — claims, votes, and releases
    // are coordination actions that don't prove models produced code changes.
    assert_eq!(council.turns_since_progress, 0);
}

#[test]
fn parse_validate_duplicate_vote_not_double_counted() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run_id = council.run_id.clone();
    council.votes_complete.push(CouncilSpeaker::Codex);

    let _ = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[VOTE_COMPLETE] still ready",
        3,
    )
    .unwrap();

    assert_eq!(council.votes_complete.len(), 1);
}

#[test]
fn parse_validate_claim_rejected_produces_rejection_entry() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[CLAIM src/lib.rs]",
        4,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::ClaimRejected));
    assert_eq!(council.active_claims.len(), 1);
    assert_eq!(council.active_claims[0].claimed_by, CouncilSpeaker::Claude);
}

#[test]
fn validate_claim_request_rejects_reclaim_of_stale_revoked_scope() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .revoked_claims
        .push(sample_revoked_claim("src/lib.rs", CouncilSpeaker::Codex));

    let err = validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::Codex)
        .expect_err("stale-revoked scope should stay blocked for same speaker");

    assert!(err
        .to_string()
        .contains("was revoked as stale earlier in this run"));
}

#[test]
fn validate_claim_request_allows_other_speaker_after_stale_revocation() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .revoked_claims
        .push(sample_revoked_claim("src/lib.rs", CouncilSpeaker::Codex));

    validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::Gemini)
        .expect("another speaker should be able to take over a stale-revoked scope");
}

#[test]
fn parse_validate_release_without_claim_produces_nudge() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[RELEASE src/nonexistent.rs]",
        5,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::SystemNudge));
    assert!(entries
        .iter()
        .any(|e| e.content.contains("no matching active claim")));
}

// ---------------------------------------------------------------------------
// Claim validation
// ---------------------------------------------------------------------------

#[test]
fn validate_claim_rejects_non_model_speaker() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    assert!(validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::System).is_err());
    assert!(validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::User).is_err());
}

#[test]
fn validate_claim_rejects_when_writes_disabled() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.allow_write_tasks = false;
    assert!(validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::Claude).is_err());
}

#[test]
fn validate_claim_rejects_empty_scope() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    assert!(validate_claim_request(&council, "", CouncilSpeaker::Claude).is_err());
    assert!(validate_claim_request(&council, "   ", CouncilSpeaker::Claude).is_err());
}

#[test]
fn validate_claim_rejects_conflicting_scope() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));
    let result = validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::Codex);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("conflicts"));
}

#[test]
fn validate_claim_allows_same_requester_to_reclaim() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));
    // Same requester can re-claim the same scope
    assert!(validate_claim_request(&council, "src/lib.rs", CouncilSpeaker::Claude).is_ok());
}

// ---------------------------------------------------------------------------
// Release claim
// ---------------------------------------------------------------------------

#[test]
fn release_claim_returns_true_when_found() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Codex));
    assert!(release_claim(
        &mut council,
        CouncilSpeaker::Codex,
        "src/lib.rs"
    ));
    assert!(council.active_claims.is_empty());
}

#[test]
fn release_claim_returns_false_for_missing_claim() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    assert!(!release_claim(
        &mut council,
        CouncilSpeaker::Codex,
        "src/lib.rs"
    ));
}

#[test]
fn release_claim_only_removes_matching_speaker_and_scope() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));
    council
        .active_claims
        .push(sample_claim("src/main.rs", CouncilSpeaker::Codex));
    // Try to release Claude's claim as Codex — should fail
    assert!(!release_claim(
        &mut council,
        CouncilSpeaker::Codex,
        "src/lib.rs"
    ));
    assert_eq!(council.active_claims.len(), 2);
    // Release Codex's own claim
    assert!(release_claim(
        &mut council,
        CouncilSpeaker::Codex,
        "src/main.rs"
    ));
    assert_eq!(council.active_claims.len(), 1);
    assert_eq!(council.active_claims[0].claimed_by, CouncilSpeaker::Claude);
}

// ---------------------------------------------------------------------------
// contains_stub_markers
// ---------------------------------------------------------------------------

#[test]
fn contains_stub_markers_detects_todo_bang() {
    let workspace = TempDir::new().unwrap();
    let marker = format!("fn main() {{ {}!() }}\n", ["to", "do"].concat());
    fs::write(workspace.path().join("stub.rs"), marker).unwrap();
    assert!(contains_stub_markers(workspace.path(), "stub.rs").unwrap());
}

#[test]
fn contains_stub_markers_detects_not_impl_bang() {
    let workspace = TempDir::new().unwrap();
    let marker = format!("fn main() {{ {}!() }}\n", ["un", "implemented"].concat());
    fs::write(workspace.path().join("stub.rs"), marker).unwrap();
    assert!(contains_stub_markers(workspace.path(), "stub.rs").unwrap());
}

#[test]
fn contains_stub_markers_detects_place_holder() {
    let workspace = TempDir::new().unwrap();
    let marker = format!(
        "// This is a {} implementation\n",
        ["place", "holder"].concat()
    );
    fs::write(workspace.path().join("stub.rs"), marker).unwrap();
    assert!(contains_stub_markers(workspace.path(), "stub.rs").unwrap());
}

#[test]
fn contains_stub_markers_clean_file_returns_false() {
    let workspace = TempDir::new().unwrap();
    fs::write(
        workspace.path().join("clean.rs"),
        "pub fn ready() { println!(\"done\"); }\n",
    )
    .unwrap();
    assert!(!contains_stub_markers(workspace.path(), "clean.rs").unwrap());
}

#[test]
fn contains_stub_markers_nonexistent_file_returns_false() {
    let workspace = TempDir::new().unwrap();
    assert!(!contains_stub_markers(workspace.path(), "does_not_exist.rs").unwrap());
}

// ---------------------------------------------------------------------------
// parse_and_validate: release flow integration
// ---------------------------------------------------------------------------

#[test]
fn parse_validate_release_does_not_reset_stall_counter() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Codex));
    council.turns_since_progress = 4;
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Codex,
        "[RELEASE src/lib.rs]",
        6,
    )
    .unwrap();

    assert!(council.active_claims.is_empty());
    // Release is a coordination action, not validated work — it does not
    // reset the stall counter. Only work reports do.
    assert_eq!(council.turns_since_progress, 5);
    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::ReleaseScope));
}

// ---------------------------------------------------------------------------
// parse_and_validate: discussion-only with mentions creates delegation
// ---------------------------------------------------------------------------

#[test]
fn parse_validate_discussion_with_mention_creates_delegation() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let run_id = council.run_id.clone();

    let entries = parse_and_validate_council_response(
        &run_id,
        &mut council,
        CouncilSpeaker::Claude,
        "@codex please implement the feature",
        7,
    )
    .unwrap();

    let kinds: Vec<CouncilEntryKind> = entries.iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&CouncilEntryKind::Discussion));
    assert!(kinds.contains(&CouncilEntryKind::Delegation));
    let delegation = entries
        .iter()
        .find(|e| e.kind == CouncilEntryKind::Delegation)
        .unwrap();
    assert!(delegation.mentions.contains(&"codex".to_string()));
}

// ---------------------------------------------------------------------------
// verify_completion edge cases
// ---------------------------------------------------------------------------

#[test]
fn verify_completion_reports_zero_work_reports() {
    let workspace = TempDir::new().unwrap();
    let council = sample_council(&workspace, TurnPolicy::RoundRobin);
    let issues = verify_completion(&council.run_id, &council).unwrap();
    assert!(issues
        .iter()
        .any(|i| i.contains("no validated work reports")));
}

#[test]
fn verify_completion_reports_active_claims() {
    let workspace = TempDir::new().unwrap();
    let mut council = sample_council(&workspace, TurnPolicy::RoundRobin);
    council.total_work_reports = 1;
    council
        .active_claims
        .push(sample_claim("src/lib.rs", CouncilSpeaker::Claude));
    let issues = verify_completion(&council.run_id, &council).unwrap();
    assert!(issues.iter().any(|i| i.contains("still claimed")));
}
