pub fn load_run(run_id: &str) -> Result<CoordinationRun> {
    let raw = fs::read_to_string(run_path(run_id))
        .with_context(|| format!("failed to read run {}", run_id))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse run {}", run_id))
}
pub fn load_task(run_id: &str, task_id: &str) -> Result<CoordinationTask> {
    let raw = fs::read_to_string(task_path(run_id, task_id))
        .with_context(|| format!("failed to read task {} for run {}", task_id, run_id))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse task {} for run {}", task_id, run_id))
}
pub fn load_claim(run_id: &str, task_id: &str) -> Result<TaskClaim> {
    let raw = fs::read_to_string(claim_path(run_id, task_id))
        .with_context(|| format!("failed to read claim {} for run {}", task_id, run_id))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse claim {} for run {}", task_id, run_id))
}
