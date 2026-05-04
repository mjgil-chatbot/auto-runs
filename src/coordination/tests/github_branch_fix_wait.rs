fn run_git_for_branch_fix_test(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
fn init_git_repo_for_branch_fix_test(workspace: &std::path::Path) {
    run_git_for_branch_fix_test(workspace, &["init", "-b", "main"]);
    run_git_for_branch_fix_test(workspace, &["config", "user.name", "Test User"]);
    run_git_for_branch_fix_test(workspace, &["config", "user.email", "test@example.com"]);
    run_git_for_branch_fix_test(workspace, &["add", "."]);
    run_git_for_branch_fix_test(workspace, &["commit", "-m", "Initial commit"]);
}
