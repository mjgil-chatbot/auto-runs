use super::parsing_actions::*;
use super::*;
use crate::coordination;
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
