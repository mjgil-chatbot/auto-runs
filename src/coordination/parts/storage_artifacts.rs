pub fn artifacts_dir(run_id: &str) -> PathBuf {
    run_dir(run_id).join("artifacts")
}
pub fn artifact_dir(run_id: &str, task_id: &str) -> PathBuf {
    artifacts_dir(run_id).join(task_id)
}
pub fn artifact_materialized_files_path(run_id: &str, task_id: &str) -> PathBuf {
    artifact_dir(run_id, task_id).join("materialized-files.json")
}
