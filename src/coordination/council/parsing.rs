use super::parsing_actions::*;
use super::*;
use crate::coordination;

pub(super) fn revoke_stale_claims(
    council: &mut CouncilRun,
    now: DateTime<Utc>,
) -> Vec<CouncilEntry> {
    let ttl = chrono::Duration::seconds(STALE_CLAIM_TTL_SECS);
    let mut stale = Vec::new();
    let mut still_active = Vec::with_capacity(council.active_claims.len());

    for claim in council.active_claims.drain(..) {
        if now.signed_duration_since(claim.claimed_at) >= ttl {
            stale.push(claim);
        } else {
            still_active.push(claim);
        }
    }
    council.active_claims = still_active;

    stale.into_iter()
        .map(|claim| {
            if !council.revoked_claims.iter().any(|revoked| {
                revoked.scope == claim.scope
                    && revoked.claimed_by == claim.claimed_by
                    && revoked.reason == ClaimRevocationReason::StaleTimeout
            }) {
                council.revoked_claims.push(RevokedClaim {
                    scope: claim.scope.clone(),
                    claimed_by: claim.claimed_by,
                    revoked_at: now,
                    reason: ClaimRevocationReason::StaleTimeout,
                });
            }

            let mut entry = CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::ClaimRevoked,
                format!(
                    "Claim revoked as stale after {}s: {} -> {}. {} may not reclaim this scope again in this run.",
                    STALE_CLAIM_TTL_SECS,
                    claim.claimed_by.label(),
                    claim.scope,
                    claim.claimed_by.label(),
                ),
            );
            entry.claims = vec![claim];
            entry
        })
        .collect()
}

pub(super) fn parse_and_validate_council_response(
    run_id: &str,
    council: &mut CouncilRun,
    speaker: CouncilSpeaker,
    response: &str,
    instance_pid: u32,
) -> Result<Vec<CouncilEntry>> {
    let mut entries = Vec::new();
    let mut made_progress = false;
    // Validated work resets the stall counter. A granted claim also resets it because the
    // claiming model needs (num_models - 1) more round-robin turns before it can file its
    // work report — without this, the stall fires before the model gets back. Votes alone
    // do NOT reset the counter, to prevent a vote loop from masking a stuck council.
    let mut real_progress = false;
    let mut claim_granted = false;
    let clean_content = strip_action_markers(response);
    let mentions = extract_mentions(response);

    let existing_ledger = super::runtime::load_council_ledger(run_id).unwrap_or_default();
    if pending_handoff_target(&existing_ledger) == Some(speaker) {
        entries.push(CouncilEntry::new(
            CouncilSpeaker::System,
            CouncilEntryKind::HandoffAccepted,
            format!("{} accepted handoff.", speaker.label()),
        ));
        made_progress = true;
    }

    if !clean_content.trim().is_empty() {
        let mut discussion =
            CouncilEntry::new(speaker, CouncilEntryKind::Discussion, clean_content.trim());
        discussion.mentions = mentions.clone();
        entries.push(discussion);
    }

    if !mentions.is_empty() && clean_content.contains('@') {
        let mut delegation =
            CouncilEntry::new(speaker, CouncilEntryKind::Delegation, clean_content.trim());
        delegation.mentions = mentions.clone();
        entries.push(delegation);
    }

    for scope in extract_claims(response) {
        let claim = WriteClaim {
            scope: scope.clone(),
            claimed_by: speaker,
            claimed_at: Utc::now(),
        };
        let mut request = CouncilEntry::new(
            speaker,
            CouncilEntryKind::ClaimRequest,
            format!("Requesting write access to {}", scope),
        );
        request.claims = vec![claim.clone()];
        request.mentions = mentions.clone();
        entries.push(request);

        match validate_claim_request(council, &scope, speaker) {
            Ok(()) => {
                if !council
                    .active_claims
                    .iter()
                    .any(|active| active.claimed_by == speaker && active.scope == scope)
                {
                    council.active_claims.push(claim.clone());
                }
                let mut granted = CouncilEntry::new(
                    CouncilSpeaker::System,
                    CouncilEntryKind::ClaimGranted,
                    format!("Claim granted: {} -> {}", speaker.label(), scope),
                );
                granted.claims = vec![claim];
                entries.push(granted);
                made_progress = true;
                claim_granted = true;
            }
            Err(reason) => {
                entries.push(CouncilEntry::new(
                    CouncilSpeaker::System,
                    CouncilEntryKind::ClaimRejected,
                    format!("Claim rejected for {}: {}", scope, reason),
                ));
            }
        }
    }

    for scope in extract_releases(response) {
        let released = release_claim(council, speaker, &scope);
        if released {
            let mut entry = CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::ReleaseScope,
                format!("Scope released: {} <- {}", scope, speaker.label()),
            );
            entry.claims.push(WriteClaim {
                scope,
                claimed_by: speaker,
                claimed_at: Utc::now(),
            });
            entries.push(entry);
            made_progress = true;
        } else {
            entries.push(CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::SystemNudge,
                format!(
                    "{} tried to release {}, but no matching active claim was found.",
                    speaker.label(),
                    scope
                ),
            ));
        }
    }

    if let Some(work_report) = extract_work_report(response, &council.workspace_path, speaker) {
        let mut report_entry = CouncilEntry::new(
            speaker,
            CouncilEntryKind::WorkReport,
            work_report.summary.clone(),
        );
        report_entry.diffs = work_report.diffs.clone();
        entries.push(report_entry);

        let validation_errors = validate_work_report(council, speaker, &work_report.changed_files)?;
        if validation_errors.is_empty() {
            council.total_work_reports = council.total_work_reports.saturating_add(1);
            store_work_artifact(run_id, council, speaker, &work_report, instance_pid)?;
            entries.push(CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::WorkValidated,
                format!(
                    "Work validated for {}: {} file(s) changed, within scope, no blocked stub markers.",
                    speaker.label(),
                    work_report.changed_files.len()
                ),
            ));
            made_progress = true;
            real_progress = true;
        } else {
            entries.push(CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::WorkValidationFailed,
                format!(
                    "Work validation failed for {}: {}",
                    speaker.label(),
                    validation_errors.join("; ")
                ),
            ));
        }
    }

    for handoff in extract_handoffs(response) {
        if let Some(target) = CouncilSpeaker::from_name(&handoff.target).filter(|s| s.is_model()) {
            let mut entry = CouncilEntry::new(
                speaker,
                CouncilEntryKind::HandoffRequested,
                format!("Handoff to {}: {}", target.label(), handoff.reason.trim()),
            );
            entry.mentions = vec![target.mention_name().to_string()];
            entries.push(entry);
            made_progress = true;
        }
    }

    for prompt in extract_user_requests(response) {
        let entry_id = Uuid::new_v4().to_string();
        let request_entry = CouncilEntry {
            entry_id: entry_id.clone(),
            at: Utc::now(),
            speaker,
            kind: CouncilEntryKind::UserInputRequested,
            content: prompt.clone(),
            mentions: vec!["user".to_string()],
            claims: Vec::new(),
            diffs: Vec::new(),
        };
        entries.push(request_entry);
        if let Err(err) =
            coordination::create_council_user_reply_interrupt(run_id, &entry_id, &prompt)
        {
            entries.push(CouncilEntry::new(
                CouncilSpeaker::System,
                CouncilEntryKind::SystemNudge,
                format!("Failed to persist council user-reply interrupt: {err}"),
            ));
        }
    }

    if response.contains("[VOTE_COMPLETE]") {
        let justification = extract_vote_justification(response, &clean_content);
        if !council.votes_complete.contains(&speaker) {
            council.votes_complete.push(speaker);
            made_progress = true;
        }
        // Auto-release all claims held by this speaker when they vote complete.
        // This prevents the deadlock where all models vote with active claims,
        // causing verify_completion to fail due to non-empty active_claims,
        // clearing all votes, and creating an infinite vote-verify-clear loop.
        let released: Vec<String> = council
            .active_claims
            .iter()
            .filter(|c| c.claimed_by == speaker)
            .map(|c| c.scope.clone())
            .collect();
        for scope in &released {
            release_claim(council, speaker, scope);
        }
        if !released.is_empty() {
            for scope in &released {
                entries.push(CouncilEntry::new(
                    CouncilSpeaker::System,
                    CouncilEntryKind::ReleaseScope,
                    format!(
                        "Scope auto-released on vote complete: {} <- {}",
                        scope,
                        speaker.label()
                    ),
                ));
            }
        }
        entries.push(CouncilEntry::new(
            speaker,
            CouncilEntryKind::VoteComplete,
            justification,
        ));
    }

    if real_progress || claim_granted {
        council.turns_since_progress = 0;
    } else {
        council.turns_since_progress = council.turns_since_progress.saturating_add(1);
    }

    Ok(entries)
}

pub(super) fn validate_claim_request(
    council: &CouncilRun,
    scope: &str,
    requester: CouncilSpeaker,
) -> Result<()> {
    if !requester.is_model() {
        bail!("only model speakers can claim write scopes");
    }
    if !council.allow_write_tasks {
        bail!("write tasks are disabled for this council run");
    }
    if scope.trim().is_empty() {
        bail!("claim scope cannot be empty");
    }
    for revoked in &council.revoked_claims {
        if revoked.claimed_by == requester
            && revoked.reason == ClaimRevocationReason::StaleTimeout
            && coordination::scope_conflicts(&revoked.scope, scope)
        {
            bail!(
                "scope '{}' was revoked as stale earlier in this run and cannot be reclaimed by {}",
                revoked.scope,
                requester.label()
            );
        }
    }

    for active in &council.active_claims {
        if active.claimed_by == requester {
            continue;
        }
        if coordination::scope_conflicts(&active.scope, scope) {
            bail!(
                "scope '{}' conflicts with '{}' held by {}",
                scope,
                active.scope,
                active.claimed_by.label()
            );
        }
    }

    Ok(())
}

pub(super) fn release_claim(
    council: &mut CouncilRun,
    speaker: CouncilSpeaker,
    scope: &str,
) -> bool {
    let before = council.active_claims.len();
    council
        .active_claims
        .retain(|claim| !(claim.claimed_by == speaker && claim.scope == scope));
    before != council.active_claims.len()
}

fn validate_work_report(
    council: &CouncilRun,
    speaker: CouncilSpeaker,
    changed_files: &[String],
) -> Result<Vec<String>> {
    let mut errors = Vec::new();
    let root = Path::new(&council.workspace_path);
    let speaker_claims: Vec<&str> = council
        .active_claims
        .iter()
        .filter(|claim| claim.claimed_by == speaker)
        .map(|claim| claim.scope.as_str())
        .collect();

    if changed_files.is_empty() {
        errors.push("no changed files were listed in the work report".to_string());
        return Ok(errors);
    }

    for file in changed_files {
        // Reject parent-directory traversal before any scope or disk checks (finding 7).
        if file.contains("..") {
            errors.push(format!(
                "'{}' contains parent-directory traversal and is rejected",
                file
            ));
            continue;
        }

        let Some(relative): Option<String> = coordination::workspace_relative_path(root, file)
        else {
            errors.push(format!("'{}' is not inside the workspace root", file));
            continue;
        };

        // After relativization, verify the resolved path stays under the workspace root.
        {
            let resolved = root.join(&relative);
            if let (Ok(canon_root), Ok(canon_resolved)) =
                (root.canonicalize(), resolved.canonicalize())
            {
                if !canon_resolved.starts_with(&canon_root) {
                    errors.push(format!(
                        "'{}' resolves outside the workspace root",
                        relative
                    ));
                    continue;
                }
            }
            // For files that don't exist yet, fall through to the existing existence check below.
        }

        if !speaker_claims
            .iter()
            .any(|scope| coordination::scope_allows_relative_path(scope, &relative))
        {
            errors.push(format!(
                "'{}' is outside {}'s claimed scopes",
                relative,
                speaker.label()
            ));
        }

        let candidate = root.join(&relative);
        if !candidate.exists() {
            errors.push(format!("'{}' does not exist on disk", relative));
            continue;
        }

        if contains_stub_markers(root, &relative)? {
            errors.push(format!(
                "'{}' still contains blocked stub markers",
                relative
            ));
        }
    }

    Ok(errors)
}

pub(super) fn contains_stub_markers(root: &Path, changed_file: &str) -> Result<bool> {
    crate::coordination::file_contains_stub_markers(root, changed_file)
}

pub(super) fn verify_completion(run_id: &str, council: &CouncilRun) -> Result<Vec<String>> {
    let mut issues = Vec::new();

    // Active claims are a hard blocker — claims must be released before completion.
    // This is the most common failure mode and the one most fixable by the models.
    if !council.active_claims.is_empty() {
        issues.push(format!(
            "{} scope(s) are still claimed and must be released before completing",
            council.active_claims.len()
        ));
    }
    // At least one validated work report must exist — no work means goal cannot be met.
    if council.total_work_reports == 0 {
        issues.push("no validated work reports were recorded".to_string());
    }
    // Empty completion criteria is informational — the concrete checks above
    // (active claims, work reports, artifact outcomes) are the authoritative
    // blockers. Don't push an issue here; a run with real work and no remaining
    // claims satisfies the intent even when criteria was left blank.

    let artifacts_root = coordination::artifacts_dir(run_id);
    if artifacts_root.is_dir() {
        let entries = fs::read_dir(&artifacts_root)
            .with_context(|| format!("failed to read {}", artifacts_root.display()))?;
        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            let result_path = entry.path().join("result.json");
            if !result_path.is_file() {
                continue;
            }
            let raw = fs::read_to_string(&result_path)
                .with_context(|| format!("failed to read {}", result_path.display()))?;
            let artifact: coordination::TaskArtifact = serde_json::from_str(&raw)
                .with_context(|| format!("failed to parse {}", result_path.display()))?;
            if !artifact.report.validation_errors.is_empty() {
                issues.push(format!(
                    "artifact {} still has validation errors",
                    artifact.task_id
                ));
            }
            if matches!(
                artifact.report.outcome,
                coordination::CoordinationTaskOutcome::Blocked
                    | coordination::CoordinationTaskOutcome::Partial
                    | coordination::CoordinationTaskOutcome::Failed
            ) {
                issues.push(format!(
                    "artifact {} ended with outcome {}",
                    artifact.task_id,
                    artifact.report.outcome.as_str()
                ));
            }
        }
    }

    Ok(issues)
}

fn store_work_artifact(
    run_id: &str,
    council: &CouncilRun,
    speaker: CouncilSpeaker,
    work_report: &ParsedWorkReport,
    instance_pid: u32,
) -> Result<()> {
    let task_id = format!(
        "council-{}-{}",
        speaker.mention_name(),
        Uuid::new_v4().simple()
    );
    let body = format!(
        "{}\n\nChanged files:\n{}",
        work_report.summary,
        work_report
            .changed_files
            .iter()
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let artifact = coordination::TaskArtifact {
        run_id: council.run_id.clone(),
        task_id: task_id.clone(),
        provider: None,
        task_category: None,
        fallback_from: None,
        summary: format!("{} reported validated council work.", speaker.label()),
        body,
        completed_at: Utc::now(),
        report: coordination::TaskExecutionReport {
            outcome: coordination::CoordinationTaskOutcome::CompletedWithChanges,
            summary: format!("Validated work report from {}.", speaker.label()),
            details: work_report.summary.clone(),
            changed_files: work_report.changed_files.clone(),
            warnings: vec![format!(
                "Recorded from council turn driven by instance PID {instance_pid}."
            )],
            design_requirements_satisfied: vec![
                "Recorded validated council work in the shared artifacts directory.".to_string(),
            ],
            ..Default::default()
        },
    };
    coordination::write_json_atomic(
        &coordination::artifact_result_path(run_id, &task_id),
        &artifact,
    )?;
    Ok(())
}

pub(super) fn extract_mentions(text: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for token in text.split_whitespace() {
        if !token.starts_with('@') {
            continue;
        }
        let normalized = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '@')
            .trim_start_matches('@')
            .to_ascii_lowercase();
        if CouncilSpeaker::from_name(&normalized)
            .map(|speaker| speaker.is_model())
            .unwrap_or(false)
            && seen.insert(normalized.clone())
        {
            mentions.push(normalized);
        }
    }
    mentions
}

pub(super) fn extract_claims(text: &str) -> Vec<String> {
    extract_bracketed_values(text, "CLAIM")
}

pub(super) fn extract_releases(text: &str) -> Vec<String> {
    extract_bracketed_values(text, "RELEASE")
}

pub(super) fn extract_revoke_votes(text: &str) -> Vec<CouncilSpeaker> {
    extract_bracketed_values(text, "REVOKE_VOTE")
        .into_iter()
        .filter_map(|value| CouncilSpeaker::from_name(&value))
        .filter(|speaker| speaker.is_model())
        .collect()
}

fn extract_bracketed_values(text: &str, action: &str) -> Vec<String> {
    let mut values = Vec::new();
    let needle = format!("[{} ", action);
    let mut cursor = text;
    while let Some(start) = cursor.find(&needle) {
        let after = &cursor[start + needle.len()..];
        let Some(end) = after.find(']') else {
            break;
        };
        let value = after[..end].trim();
        if !value.is_empty() {
            values.push(value.to_string());
        }
        cursor = &after[end + 1..];
    }
    values
}

pub(super) fn extract_work_report(
    response: &str,
    workspace_path: &str,
    speaker: CouncilSpeaker,
) -> Option<ParsedWorkReport> {
    let idx = response.find("[WORK_DONE]")?;
    let after = response[idx + "[WORK_DONE]".len()..].trim();
    let root = Path::new(workspace_path);
    let mut changed_files = Vec::new();
    let mut diffs = Vec::new();
    let mut summary_lines = Vec::new();

    for raw_line in after.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }

        let inline_changed = extract_bracketed_values(line, "CHANGED");
        if !inline_changed.is_empty() {
            for file in inline_changed {
                changed_files.push(file.clone());
                diffs.push(DiffSummary {
                    file,
                    lines_added: 0,
                    lines_removed: 0,
                    description: format!("{} marked file as changed.", speaker.label()),
                });
            }
            continue;
        }

        if let Some(diff) = parse_diff_summary_line(line, root) {
            changed_files.push(diff.file.clone());
            diffs.push(diff);
            continue;
        }

        let path_candidates = extract_changed_file_candidates(line, root);
        if !path_candidates.is_empty() {
            for file in path_candidates {
                changed_files.push(file.clone());
                diffs.push(DiffSummary {
                    file,
                    lines_added: 0,
                    lines_removed: 0,
                    description: format!("{} reported the file as changed.", speaker.label()),
                });
            }
            continue;
        }

        summary_lines.push(line.to_string());
    }

    let mut seen = HashSet::new();
    changed_files = changed_files
        .into_iter()
        .filter(|file| !file.contains(".."))
        .filter_map(|file| coordination::workspace_relative_path(root, &file))
        .filter(|file: &String| seen.insert(file.clone()))
        .collect::<Vec<String>>();
    diffs.retain(|diff| seen.contains(&normalize_candidate_path(root, &diff.file)));
    if diffs.is_empty() {
        diffs = changed_files
            .iter()
            .map(|file| DiffSummary {
                file: file.clone(),
                lines_added: 0,
                lines_removed: 0,
                description: format!("{} reported the file as changed.", speaker.label()),
            })
            .collect();
    } else {
        for diff in &mut diffs {
            diff.file = normalize_candidate_path(root, &diff.file);
        }
    }

    let summary = if summary_lines.is_empty() {
        format!("{} reported completed work.", speaker.label())
    } else {
        summary_lines.join("\n")
    };

    Some(ParsedWorkReport {
        summary,
        changed_files,
        diffs,
    })
}

fn normalize_candidate_path(root: &Path, candidate: &str) -> String {
    coordination::workspace_relative_path(root, candidate).unwrap_or_else(|| candidate.to_string())
}

fn parse_diff_summary_line(line: &str, root: &Path) -> Option<DiffSummary> {
    let parts: Vec<&str> = line.split('|').map(str::trim).collect();
    if parts.len() < 2 {
        return None;
    }
    let file = parts[0].trim_matches('`');
    let candidate = Path::new(file);
    let exists = if candidate.is_absolute() {
        candidate.exists()
    } else {
        root.join(candidate).exists()
    };
    if !exists {
        return None;
    }
    let (lines_added, lines_removed) = parse_diff_counts(parts[1]);
    let description = parts.get(2).copied().unwrap_or("reported diff summary");
    Some(DiffSummary {
        file: file.to_string(),
        lines_added,
        lines_removed,
        description: description.to_string(),
    })
}

fn parse_diff_counts(segment: &str) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for token in segment.split_whitespace() {
        if let Some(value) = token.strip_prefix('+') {
            added = value.parse().unwrap_or(0);
        } else if let Some(value) = token.strip_prefix('-') {
            removed = value.parse().unwrap_or(0);
        } else if let Ok(value) = token.parse::<usize>() {
            // Bare numbers (no sign prefix) are treated as added lines.
            // This handles model output like "| 10" where the sign was omitted.
            added = added.saturating_add(value);
        }
    }
    (added, removed)
}

fn extract_changed_file_candidates(line: &str, root: &Path) -> Vec<String> {
    let trimmed = line.trim();
    let lower = trimmed.to_ascii_lowercase();

    let candidates_str = if let Some(rest) = lower
        .strip_prefix("changed files:")
        .or_else(|| lower.strip_prefix("changed file:"))
        .or_else(|| lower.strip_prefix("files:"))
    {
        let offset = trimmed.len() - rest.len();
        trimmed[offset..].trim().to_string()
    } else if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
    {
        rest.trim().to_string()
    } else {
        trimmed.to_string()
    };

    candidates_str
        .split(',')
        .map(|value| value.trim())
        .filter_map(|value| clean_path_candidate(value, root))
        .collect()
}

fn clean_path_candidate(value: &str, root: &Path) -> Option<String> {
    let first = value
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches('`')
        .trim_matches(|ch: char| matches!(ch, ':' | ';' | ',' | '(' | ')' | '[' | ']'));
    if first.is_empty() || first.starts_with('[') || first.starts_with('@') {
        return None;
    }
    let candidate = Path::new(first);
    let exists = if candidate.is_absolute() {
        candidate.exists()
    } else {
        root.join(candidate).exists()
    };
    exists.then(|| first.to_string())
}

pub(super) fn extract_vote_justification(response: &str, clean_content: &str) -> String {
    if let Some(idx) = response.find("[VOTE_COMPLETE]") {
        let after_marker = &response[idx + "[VOTE_COMPLETE]".len()..];
        let same_line = match after_marker.find('\n') {
            Some(nl) => &after_marker[..nl],
            None => after_marker,
        };
        let stripped = strip_protocol_markers_inline(same_line);
        if !stripped.is_empty() {
            return stripped;
        }
    }
    if !clean_content.trim().is_empty() {
        clean_content.trim().to_string()
    } else {
        "Speaker voted complete.".to_string()
    }
}

fn strip_protocol_markers_inline(text: &str) -> String {
    let mut result = text.to_string();
    for prefix in &["[CLAIM ", "[RELEASE "] {
        while let Some(start) = result.find(prefix) {
            if let Some(end) = result[start..].find(']') {
                result.replace_range(start..start + end + 1, "");
            } else {
                break;
            }
        }
    }
    for marker in &["[WORK_DONE]", "[VOTE_COMPLETE]"] {
        result = result.replace(marker, "");
    }
    result.trim().to_string()
}
