use super::parsing_actions::*;
use super::*;
use crate::coordination;
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
