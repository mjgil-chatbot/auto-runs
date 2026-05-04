#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[test]
fn github_branch_fix_notes_require_branch() {
    let notes = github_branch_fix_notes_with_settings("", "", 5);
    let error =
        validate_github_branch_fix_notes(&notes).expect_err("branchless config should fail");
    assert!(error.to_string().contains("github_branch_fix_branch"));
}
