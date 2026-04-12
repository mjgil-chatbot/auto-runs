use super::*;
pub fn extract_first_json_object(body: &str) -> Option<&str> {
    extract_balanced_json(body, '{', '}')
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
pub fn normalize_provider_name(provider: Option<&str>) -> Option<String> {
    let provider = provider?.trim();
    match provider.to_ascii_lowercase().as_str() {
        "glm" => Some("glm".to_string()),
        "nemotron" | "nvidia" => Some("nemotron".to_string()),
        _ => ModelProvider::from_arg(provider).map(|provider| provider.as_str().to_string()),
    }
}
