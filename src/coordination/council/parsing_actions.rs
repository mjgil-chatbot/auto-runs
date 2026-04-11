use super::*;

pub(super) struct ParsedHandoff {
    pub target: String,
    pub reason: String,
}

pub(super) fn extract_handoffs(text: &str) -> Vec<ParsedHandoff> {
    let mut out = Vec::new();
    let needle = "[HANDOFF ";
    let mut cursor = text;
    while let Some(start) = cursor.find(needle) {
        let after = &cursor[start + needle.len()..];
        let Some(end) = after.find(']') else {
            break;
        };
        let inner = after[..end].trim();
        let mut parts = inner.splitn(2, char::is_whitespace);
        if let Some(target) = parts.next() {
            let reason = parts.next().unwrap_or("").trim();
            if !target.is_empty() {
                out.push(ParsedHandoff {
                    target: target.trim_start_matches('@').to_string(),
                    reason: reason.to_string(),
                });
            }
        }
        cursor = &after[end + 1..];
    }
    out
}

pub(super) fn extract_user_requests(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let needle = "[REQUEST_USER ";
    let mut cursor = text;
    while let Some(start) = cursor.find(needle) {
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

pub(super) fn pending_handoff_target(ledger: &[CouncilEntry]) -> Option<CouncilSpeaker> {
    let last_handoff = ledger.iter().rev().find(|e| {
        e.kind == CouncilEntryKind::HandoffRequested || e.kind == CouncilEntryKind::HandoffAccepted
    })?;
    if last_handoff.kind == CouncilEntryKind::HandoffAccepted {
        return None;
    }
    let target_name = last_handoff.mentions.first()?;
    CouncilSpeaker::from_name(target_name).filter(|s| s.is_model())
}

pub(super) fn strip_action_markers(text: &str) -> String {
    let mut cleaned_lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "[WORK_DONE]" || trimmed == "[VOTE_COMPLETE]" {
            continue;
        }
        if trimmed.starts_with("[CLAIM ")
            || trimmed.starts_with("[RELEASE ")
            || trimmed.starts_with("[HANDOFF ")
            || trimmed.starts_with("[REQUEST_USER ")
        {
            continue;
        }
        let cleaned = trimmed
            .replace("[WORK_DONE]", "")
            .replace("[VOTE_COMPLETE]", "")
            .trim()
            .to_string();
        if !cleaned.is_empty() {
            cleaned_lines.push(cleaned);
        }
    }
    cleaned_lines.join("\n")
}

pub(super) fn pending_user_request(ledger: &[CouncilEntry]) -> bool {
    let last_request_idx = ledger
        .iter()
        .rposition(|e| e.kind == CouncilEntryKind::UserInputRequested);
    let Some(idx) = last_request_idx else {
        return false;
    };
    !ledger[idx + 1..]
        .iter()
        .any(|e| e.kind == CouncilEntryKind::UserInterjection)
}
