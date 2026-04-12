use super::*;
fn short_git_commit_display(commit: &str) -> &str {
    commit.get(..12).unwrap_or(commit)
}
