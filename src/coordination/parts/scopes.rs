use std::path::{Component, Path};
pub fn normalize_scope(scope: &str) -> String {
    let trimmed = scope.trim().trim_matches('/');
    if trimmed.is_empty() {
        ".".to_string()
    } else {
        trimmed.to_string()
    }
}
pub fn wildcard_prefix(scope: &str) -> Option<String> {
    scope
        .find('*')
        .map(|idx| scope[..idx].trim_end_matches('/').trim().to_string())
}
