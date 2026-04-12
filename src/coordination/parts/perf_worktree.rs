use super::*;
const PERF_WORKTREE_PATH_NOTE_KEY: &str = "perf_worktree_path";
pub fn perf_workspace_path_for_run(run: &CoordinationRun) -> String {
    if run.workflow_mode != AutonomousWorkflowMode::PerfHillClimber {
        return run.workspace_path.clone();
    }

    note_setting_value(&run.notes, PERF_WORKTREE_PATH_NOTE_KEY)
        .map(str::to_string)
        .unwrap_or_else(|| run.workspace_path.clone())
}
fn note_setting_value<'a>(notes: &'a str, key: &str) -> Option<&'a str> {
    notes.lines().find_map(|line| {
        let trimmed = line.trim();
        let prefix = format!("{key}=");
        trimmed.strip_prefix(&prefix).and_then(|value| {
            let value = value.trim();
            (!value.is_empty()).then_some(value)
        })
    })
}
