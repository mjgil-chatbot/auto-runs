fn cleanup_council_run(run_id: &str) {
    let _ = fs::remove_dir_all(run_dir(run_id));
}
fn pending_commit_path(run_id: &str) -> std::path::PathBuf {
    coordination::council_json_path(run_id).with_file_name("council-commit.json")
}
