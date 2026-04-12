use std::path::{Component, Path};
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
