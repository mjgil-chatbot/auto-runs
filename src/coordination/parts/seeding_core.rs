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
pub fn load_tasks(run_id: &str) -> Result<Vec<CoordinationTask>> {
    let mut tasks = Vec::new();
    let entries = match fs::read_dir(tasks_dir(run_id)) {
        Ok(entries) => entries,
        Err(_) => return Ok(tasks),
    };

    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // Ignore atomic-write scratch files like `.task.json.tmp-12345`.
        if name.starts_with('.') || !name.ends_with(".json") {
            continue;
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("failed to read task {}", path.display()))?;
        let task: CoordinationTask = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse task {}", path.display()))?;
        tasks.push(task);
    }

    tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(tasks)
}
