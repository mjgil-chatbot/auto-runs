pub fn load_run(run_id: &str) -> Result<CoordinationRun> {
    let raw = fs::read_to_string(run_path(run_id))
        .with_context(|| format!("failed to read run {}", run_id))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse run {}", run_id))
}
