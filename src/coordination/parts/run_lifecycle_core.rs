pub(crate) fn persist_selected_tasks(
    run_id: &str,
    tasks: &[CoordinationTask],
    task_ids: &[String],
) -> Result<()> {
    for task_id in task_ids {
        let Some(task) = tasks.iter().find(|task| task.task_id == *task_id) else {
            continue;
        };
        write_json_atomic(&task_path(run_id, &task.task_id), task)?;
        index_task(task);
    }
    Ok(())
}
pub fn update_run_max_agents(run_id: &str, max_agents: u32) -> Result<()> {
    let mut run = load_run(run_id)?;
    let clamped = if matches!(
        run.workflow_mode,
        AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
    ) {
        1
    } else {
        max_agents.clamp(1, 8)
    };
    if run.max_agents != clamped {
        run.max_agents = clamped;
        run.updated_at = Utc::now();
        write_json_atomic(&run_path(run_id), &run)?;
    }
    Ok(())
}
fn normalized_max_agents(workflow_mode: &AutonomousWorkflowMode, max_agents: u32) -> u32 {
    if matches!(
        workflow_mode,
        AutonomousWorkflowMode::RalphLoop | AutonomousWorkflowMode::GithubBranchFix
    ) {
        1
    } else {
        max_agents.max(1)
    }
}
/// Find the most recent coordination run whose `root_chat_id` matches the given session ID.
/// Returns the run_id if found.
pub fn find_latest_run_for_session(session_id: &str) -> Option<String> {
    let root = coordination_root_dir();
    let mut best: Option<(chrono::DateTime<Utc>, String)> = None;
    let entries = fs::read_dir(&root).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let run_file = entry.path().join("run.json");
        let Ok(data) = fs::read_to_string(&run_file) else {
            continue;
        };
        if let Ok(run) = serde_json::from_str::<CoordinationRun>(&data) {
            if run.root_chat_id == session_id {
                if best
                    .as_ref()
                    .map(|(ts, _)| run.updated_at > *ts)
                    .unwrap_or(true)
                {
                    best = Some((run.updated_at, run.run_id.clone()));
                }
            }
        }
    }
    best.map(|(_, id)| id)
}
