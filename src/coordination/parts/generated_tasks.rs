use super::*;
pub fn extract_first_json_object(body: &str) -> Option<&str> {
    extract_balanced_json(body, '{', '}')
}
pub fn extract_first_json_array(body: &str) -> Option<&str> {
    let mut fallback = None;
    let mut search_from = 0usize;

    while search_from < body.len() {
        let start = search_from + body[search_from..].find('[')?;
        let candidate = extract_balanced_json_from(body, start, '[', ']')?;
        let inner = candidate[1..].trim_start();
        if inner.starts_with('{') || inner.starts_with(']') {
            return Some(candidate);
        }
        if fallback.is_none() {
            fallback = Some(candidate);
        }
        search_from = start + 1;
    }

    fallback
}
pub fn extract_balanced_json(body: &str, open_ch: char, close_ch: char) -> Option<&str> {
    let start = body.find(open_ch)?;
    extract_balanced_json_from(body, start, open_ch, close_ch)
}
fn extract_balanced_json_from(
    body: &str,
    start: usize,
    open_ch: char,
    close_ch: char,
) -> Option<&str> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (offset, ch) in body[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            _ if ch == open_ch => depth += 1,
            _ if ch == close_ch => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let end = start + offset + ch.len_utf8();
                    return body.get(start..end);
                }
            }
            _ => {}
        }
    }

    None
}
pub fn parse_generated_kind(kind: &str) -> Result<CoordinationTaskKind> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "write" | "edit" => Ok(CoordinationTaskKind::GeneratedWriteTask),
        "read" | "research" | "search" | "review" => Ok(CoordinationTaskKind::GeneratedReadTask),
        other => anyhow::bail!(
            "unknown generated task kind '{}'; expected one of: write, edit, read, research, search, review",
            other
        ),
    }
}
pub fn normalize_provider_name(provider: Option<&str>) -> Option<String> {
    let provider = provider?.trim();
    match provider.to_ascii_lowercase().as_str() {
        "glm" => Some("glm".to_string()),
        "nemotron" | "nvidia" => Some("nemotron".to_string()),
        _ => ModelProvider::from_arg(provider).map(|provider| provider.as_str().to_string()),
    }
}
pub fn is_generated_task_kind(kind: &CoordinationTaskKind) -> bool {
    matches!(
        kind,
        CoordinationTaskKind::GeneratedReadTask | CoordinationTaskKind::GeneratedWriteTask
    )
}
