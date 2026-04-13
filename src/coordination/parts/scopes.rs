use std::path::{Component, Path};
pub fn scopes_conflict(left: &[String], right: &[String]) -> bool {
    left.iter()
        .any(|lhs| right.iter().any(|rhs| scope_conflicts(lhs, rhs)))
}
pub fn scope_conflicts(left: &str, right: &str) -> bool {
    let lhs = normalize_scope(left);
    let rhs = normalize_scope(right);
    lhs == "."
        || rhs == "."
        || lhs == "**"
        || rhs == "**"
        || lhs == rhs
        || lhs.starts_with(&(rhs.clone() + "/"))
        || rhs.starts_with(&(lhs.clone() + "/"))
        || wildcard_prefix(&lhs)
            .zip(wildcard_prefix(&rhs))
            .map(|(lp, rp)| lp.starts_with(&rp) || rp.starts_with(&lp))
            .unwrap_or(false)
        || wildcard_prefix(&lhs)
            .map(|prefix| rhs.starts_with(&prefix))
            .unwrap_or(false)
        || wildcard_prefix(&rhs)
            .map(|prefix| lhs.starts_with(&prefix))
            .unwrap_or(false)
}
pub fn normalize_scope(scope: &str) -> String {
    let trimmed = scope.trim().trim_matches('/');
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}
pub fn raw_scope_is_absolute(scope: &str) -> bool {
    let trimmed = scope.trim();
    Path::new(trimmed).is_absolute()
        || trimmed.starts_with(['\\', '/'])
        || trimmed
            .chars()
            .next()
            .zip(trimmed.chars().nth(1))
            .is_some_and(|(first, second)| first.is_ascii_alphabetic() && second == ':')
}
pub fn wildcard_prefix(scope: &str) -> Option<String> {
    scope
        .find('*')
        .map(|idx| scope[..idx].trim_end_matches('/').trim().to_string())
}
