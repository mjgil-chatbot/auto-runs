use super::*;
#[derive(Default)]
pub(super) struct RestartDialogState {
    pub(super) open: bool,
    pub(super) selected_task_id: String,
    pub(super) mark_selected_task_failed: bool,
    pub(super) assigned_provider: String,
    pub(super) reason: String,
}
impl AutoRunsApp {
    pub(super) fn restart_dialog_tasks(
        &self,
    ) -> Vec<(String, String, CoordinationTaskStatus, Option<String>)> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .tasks
                    .iter()
                    .filter(|task| {
                        matches!(
                            task.status,
                            CoordinationTaskStatus::Failed
                                | CoordinationTaskStatus::Canceled
                                | CoordinationTaskStatus::Completed
                        )
                    })
                    .map(|task| {
                        (
                            task.task_id.clone(),
                            task.title.clone(),
                            task.status,
                            task.assigned_provider.clone(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }}
impl AutoRunsApp {
    pub(super) fn open_restart_dialog(&mut self) {
        if self.restart_dialog.reason.trim().is_empty() {
            self.restart_dialog.reason = "User requested restart.".to_string();
        }
        self.restart_dialog.open = true;
        let tasks = self.restart_dialog_tasks();
        if !tasks
            .iter()
            .any(|(task_id, _, _, _)| *task_id == self.restart_dialog.selected_task_id)
        {
            self.restart_dialog.selected_task_id = tasks
                .first()
                .map(|(task_id, _, _, _)| task_id.clone())
                .unwrap_or_default();
        }
        if let Some((_, _, status, _)) = tasks
            .iter()
            .find(|(task_id, _, _, _)| *task_id == self.restart_dialog.selected_task_id)
        {
            self.restart_dialog.mark_selected_task_failed =
                *status == CoordinationTaskStatus::Completed;
        }
    }}
