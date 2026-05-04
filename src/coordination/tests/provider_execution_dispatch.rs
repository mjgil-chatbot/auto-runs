fn run_git_for_test(workspace: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {:?} should succeed", args);
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}
