use super::parsing_actions::*;
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CouncilTerminationOutcome {
    Complete,
    Fail(&'static str),
    Cancel,
}

pub(super) fn check_stall(turns_since_progress: u32) -> Option<StallAction> {
    match turns_since_progress {
        FIRM_STALL_TURNS..TERMINAL_STALL_TURNS => Some(StallAction::FirmNudge),
        TERMINAL_STALL_TURNS.. => Some(StallAction::FirmNudge),
        SOFT_STALL_TURNS..FIRM_STALL_TURNS => Some(StallAction::SoftNudge),
        _ => None,
    }
}

pub(super) fn evaluate_termination(
    council: &CouncilRun,
    run_status: crate::coordination::CoordinationRunStatus,
) -> Option<CouncilTerminationOutcome> {
    let mut all_satisfied = !council.termination_policy.all.is_empty();
    for condition in &council.termination_policy.all {
        if !condition_met(council, run_status, condition) {
            all_satisfied = false;
            break;
        }
    }
    if all_satisfied {
        return Some(CouncilTerminationOutcome::Complete);
    }
    for condition in &council.termination_policy.any {
        if condition_met(council, run_status, condition) {
            return Some(match condition {
                TerminationCondition::UserStop => CouncilTerminationOutcome::Cancel,
                _ => CouncilTerminationOutcome::Fail(condition_label(condition)),
            });
        }
    }
    None
}

fn condition_met(
    council: &CouncilRun,
    run_status: crate::coordination::CoordinationRunStatus,
    condition: &TerminationCondition,
) -> bool {
    match condition {
        TerminationCondition::MaxTurns(limit) => council.current_turn >= *limit,
        TerminationCondition::MaxStallTurns(limit) => council.turns_since_progress >= *limit,
        TerminationCondition::AllModelsVotedComplete => {
            council.votes_complete.len() == CouncilSpeaker::MODEL_ORDER.len()
        }
        TerminationCondition::BudgetExceeded => false,
        TerminationCondition::UserStop => {
            run_status == crate::coordination::CoordinationRunStatus::Canceled
                || council.status == CouncilRunStatus::Canceled
        }
    }
}

fn condition_label(condition: &TerminationCondition) -> &'static str {
    match condition {
        TerminationCondition::MaxTurns(_) => "max-turns",
        TerminationCondition::MaxStallTurns(_) => "max-stall-turns",
        TerminationCondition::AllModelsVotedComplete => "all-voted-complete",
        TerminationCondition::BudgetExceeded => "budget-exceeded",
        TerminationCondition::UserStop => "user-stop",
    }
}

pub(super) fn apply_stall_action(
    _run_id: &str,
    council: &mut CouncilRun,
    action: StallAction,
) -> Vec<CouncilEntry> {
    match action {
        StallAction::SoftNudge => vec![CouncilEntry::new(
            CouncilSpeaker::System,
            CouncilEntryKind::SystemNudge,
            format!(
                "The council has debated for {} turns without action. Consider claiming files, releasing scopes, reporting work, or voting complete.",
                council.turns_since_progress
            ),
        )],
        StallAction::FirmNudge => vec![CouncilEntry::new(
            CouncilSpeaker::System,
            CouncilEntryKind::SystemNudge,
            "No claims or validated work in 6 turns. Each model must either claim a scope, report work, release a scope, or vote complete on its next turn.",
        )],
    }
}

pub(super) fn select_next_speaker(
    council: &CouncilRun,
    ledger: &[CouncilEntry],
) -> Option<CouncilSpeaker> {
    if let Some(handoff_target) = pending_handoff_target(ledger) {
        return Some(handoff_target);
    }
    if pending_user_request(ledger) {
        return None;
    }
    match council.turn_policy {
        TurnPolicy::RoundRobin => Some(next_in_rotation(last_model_speaker(ledger))),
        TurnPolicy::MentionDriven => select_mention_driven_speaker(ledger),
        TurnPolicy::Adaptive => select_adaptive_speaker(council, ledger),
    }
}

fn select_mention_driven_speaker(ledger: &[CouncilEntry]) -> Option<CouncilSpeaker> {
    let last_speaker = last_model_speaker(ledger);
    for entry in ledger.iter().rev() {
        for mention in entry.mentions.iter().rev() {
            let candidate = CouncilSpeaker::from_name(mention)?;
            if candidate.is_model() && Some(candidate) != last_speaker {
                return Some(candidate);
            }
        }
    }
    Some(next_in_rotation(last_speaker))
}

fn select_adaptive_speaker(
    council: &CouncilRun,
    ledger: &[CouncilEntry],
) -> Option<CouncilSpeaker> {
    let last_speaker = last_model_speaker(ledger);
    let most_recent_mention = most_recent_model_mention(ledger);
    let mut best = None;
    let mut best_score = i32::MIN;

    for (idx, speaker) in CouncilSpeaker::MODEL_ORDER.iter().copied().enumerate() {
        let mut score = 0i32;
        if Some(speaker) != last_speaker {
            score += 2;
        }
        if Some(speaker) == most_recent_mention {
            score += 6;
        }
        score += council
            .active_claims
            .iter()
            .filter(|claim| claim.claimed_by == speaker)
            .count() as i32
            * 3;
        score += turns_since_last_speaker_turn(ledger, speaker) as i32;
        score += idx as i32;

        if score > best_score {
            best_score = score;
            best = Some(speaker);
        }
    }

    best.or_else(|| Some(next_in_rotation(last_speaker)))
}

fn next_in_rotation(last: Option<CouncilSpeaker>) -> CouncilSpeaker {
    match last {
        Some(CouncilSpeaker::Claude) => CouncilSpeaker::Codex,
        Some(CouncilSpeaker::Codex) => CouncilSpeaker::Minimax,
        Some(CouncilSpeaker::Minimax) => CouncilSpeaker::Qwen,
        Some(CouncilSpeaker::Qwen) => CouncilSpeaker::Gemini,
        Some(CouncilSpeaker::Gemini) | None => CouncilSpeaker::Claude,
        Some(CouncilSpeaker::System | CouncilSpeaker::User) => CouncilSpeaker::Claude,
    }
}

fn last_model_speaker(ledger: &[CouncilEntry]) -> Option<CouncilSpeaker> {
    ledger
        .iter()
        .rev()
        .find(|entry| entry.speaker.is_model())
        .map(|entry| entry.speaker)
}

fn most_recent_model_mention(ledger: &[CouncilEntry]) -> Option<CouncilSpeaker> {
    ledger
        .iter()
        .rev()
        .flat_map(|entry| entry.mentions.iter().rev())
        .find_map(|mention| CouncilSpeaker::from_name(mention).filter(|speaker| speaker.is_model()))
}

fn turns_since_last_speaker_turn(ledger: &[CouncilEntry], speaker: CouncilSpeaker) -> usize {
    ledger
        .iter()
        .rev()
        .filter(|entry| entry.speaker.is_model())
        .position(|entry| entry.speaker == speaker)
        .unwrap_or(usize::MAX)
}

pub(super) fn build_council_prompt(
    council: &CouncilRun,
    ledger: &[CouncilEntry],
    speaker: CouncilSpeaker,
) -> String {
    let max_turns = max_turn_limit(council).unwrap_or(council.max_turns);
    let history = ledger
        .iter()
        .map(|entry| {
            format!(
                "[{}] {}: {}",
                entry.kind_label(),
                entry.speaker_label(),
                entry.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "You are {} in a brokered multi-model council working on:\n{}\n\n\
Council members: Claude (design/review), Codex (implementation), MiniMax (analysis), Qwen (cheap fallback), Gemini (testing/verification).\n\
You are talking directly to the other models through a shared ledger.\n\n\
Available actions:\n\
- Mention another model directly with @claude, @codex, @minimax, @qwen, or @gemini.\n\
- Explicitly delegate with [HANDOFF @claude reason], [HANDOFF @codex reason], [HANDOFF @minimax reason], [HANDOFF @qwen reason], or [HANDOFF @gemini reason]. The handoff target is selected next ahead of normal routing heuristics.\n\
- Claim a file BEFORE writing it: [CLAIM path/to/file]. The broker grants or rejects claims.\n\
  You MUST claim a file on one turn and THEN write it and report work on a later turn.\n\
  Writing a file without an active broker-granted claim will cause your work report to be rejected.\n\
- Release a scope with [RELEASE path/to/file].\n\
- Report completed work with [WORK_DONE] after you have written the file under your claim:\n\
  [WORK_DONE]\n\
  changed files: src/foo.rs, src/bar.rs\n\
  summary: wired the route and added tests\n\
- Vote complete with [VOTE_COMPLETE] and include evidence.\n\
- Request a blocking user decision with [REQUEST_USER one clear question]. The broker pauses the council until the user replies.\n\
- If the user says [REVOKE_VOTE speaker] or [REVOKE_ALL_VOTES], the broker may revoke votes.\n\n\
Guardrails:\n\
- Claim first, write second, report third — in that order across turns. Never skip the claim step.\n\
- You may only report work on files covered by your active broker-granted claims.\n\
- Claims auto-expire after 3 minutes. If the broker revokes one of your claims as stale, you may not reclaim that same scope in this run.\n\
- Work reports are rejected if files are outside your claims, missing on disk, or still contain broker-blocked stub markers.\n\
- Use [HANDOFF ...] when intentionally delegating; plain @mentions are fallback hints only.\n\
- Completion is accepted only after all council models vote complete and the broker verifies the workspace.\n\n\
Goal: {}\n\
Completion criteria: {}\n\
Active claims: {}\n\
Turn policy: {}\n\
Termination policy: {}\n\
Turn {}/{}. Current completion votes: {}/{}.\n\n\
Be direct, concise, and action-oriented. Do not restate the entire transcript.\n\n\
--- Council Transcript ---\n{}\n\n\
--- Your Turn ({}) ---",
        speaker.label(),
        council.goal.trim(),
        council.goal.trim(),
        council.completion_criteria.trim(),
        format_active_claims(&council.active_claims),
        council.turn_policy.as_str(),
        format_termination_policy(council),
        council.current_turn.saturating_add(1),
        max_turns,
        council.votes_complete.len(),
        CouncilSpeaker::MODEL_ORDER.len(),
        history,
        speaker.label()
    )
}

pub(super) fn max_turn_limit(council: &CouncilRun) -> Option<u32> {
    council.termination_policy.any.iter().find_map(|condition| {
        if let TerminationCondition::MaxTurns(limit) = condition {
            Some(*limit)
        } else {
            None
        }
    })
}

pub(super) fn max_stall_turn_limit(council: &CouncilRun) -> Option<u32> {
    council.termination_policy.any.iter().find_map(|condition| {
        if let TerminationCondition::MaxStallTurns(limit) = condition {
            Some(*limit)
        } else {
            None
        }
    })
}

pub(super) fn termination_failure_message(
    run_id: &str,
    council: &CouncilRun,
    reason: &str,
) -> String {
    match reason {
        "max-turns" => format!(
            "Council hit the termination policy max-turns limit at {}/{} for run {}.",
            council.current_turn,
            max_turn_limit(council).unwrap_or(council.max_turns),
            run_id
        ),
        "max-stall-turns" => format!(
            "Council stalled for {} turns and hit the termination policy max-stall-turns limit of {} in run {}.",
            council.turns_since_progress,
            max_stall_turn_limit(council).unwrap_or(TERMINAL_STALL_TURNS),
            run_id
        ),
        "budget-exceeded" => {
            format!("Council hit the termination policy budget limit for run {}.", run_id)
        }
        "user-stop" => format!("Council run {} stopped on user request.", run_id),
        _ => format!("Council run {} stopped by termination policy: {}.", run_id, reason),
    }
}

fn format_termination_policy(council: &CouncilRun) -> String {
    let any = council
        .termination_policy
        .any
        .iter()
        .map(format_termination_condition)
        .collect::<Vec<_>>();
    let all = council
        .termination_policy
        .all
        .iter()
        .map(format_termination_condition)
        .collect::<Vec<_>>();
    format!(
        "any: {}; all: {}",
        if any.is_empty() {
            "none".to_string()
        } else {
            any.join(", ")
        },
        if all.is_empty() {
            "none".to_string()
        } else {
            all.join(", ")
        }
    )
}

fn format_termination_condition(condition: &TerminationCondition) -> String {
    match condition {
        TerminationCondition::MaxTurns(limit) => format!("max_turns={limit}"),
        TerminationCondition::MaxStallTurns(limit) => format!("max_stall_turns={limit}"),
        TerminationCondition::AllModelsVotedComplete => "all_models_voted_complete".to_string(),
        TerminationCondition::BudgetExceeded => "budget_exceeded".to_string(),
        TerminationCondition::UserStop => "user_stop".to_string(),
    }
}

fn format_active_claims(claims: &[WriteClaim]) -> String {
    if claims.is_empty() {
        return "none".to_string();
    }
    claims
        .iter()
        .map(|claim| format!("{} -> {}", claim.claimed_by.label(), claim.scope))
        .collect::<Vec<_>>()
        .join(", ")
}
