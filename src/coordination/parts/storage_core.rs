use fs2::FileExt;
pub fn coordination_root_dir() -> PathBuf {
    config::shared_infra_dir().join("coordination")
}
pub fn run_dir(run_id: &str) -> PathBuf {
    coordination_root_dir().join(run_id)
}
/// Path to the council state file for a run.
pub(crate) fn council_json_path(run_id: &str) -> PathBuf {
    run_dir(run_id).join("council.json")
}
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("path missing parent")?;
    fs::create_dir_all(parent)?;
    let json = serde_json::to_string_pretty(value)?;
    crate::storage::atomic::atomic_write(path, json.as_bytes())?;
    Ok(())
}
