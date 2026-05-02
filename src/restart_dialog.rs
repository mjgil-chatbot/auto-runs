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
    }

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
    }

    pub(super) fn submit_restart_dialog(&mut self) {
        let Some(run_id) = self.selected_run_id.clone() else {
            return;
        };
        let tasks = self.restart_dialog_tasks();
        let selected = tasks
            .iter()
            .find(|(task_id, _, _, _)| *task_id == self.restart_dialog.selected_task_id);
        let has_failed_tasks = tasks.iter().any(|(_, _, status, _)| {
            matches!(
                status,
                CoordinationTaskStatus::Failed | CoordinationTaskStatus::Canceled
            )
        });
        let reason = self.restart_dialog.reason.trim();
        if selected.is_some_and(|(_, _, status, _)| *status == CoordinationTaskStatus::Completed)
            && !self.restart_dialog.mark_selected_task_failed
            && !has_failed_tasks
        {
            self.append_error(
                "select a failed task or mark the completed task failed before restarting",
            );
            return;
        }

        if let Some((task_id, _, status, _)) = selected {
            if !self.restart_dialog.assigned_provider.trim().is_empty() {
                if let Err(err) = coordination::set_task_assigned_provider(
                    &run_id,
                    task_id,
                    self.restart_dialog.assigned_provider.trim(),
                ) {
                    self.append_error(format!("failed to update task provider: {err}"));
                    return;
                }
            }
            if *status == CoordinationTaskStatus::Completed
                && self.restart_dialog.mark_selected_task_failed
            {
                if let Err(err) = coordination::mark_task_failed_for_restart(
                    &run_id,
                    task_id,
                    if reason.is_empty() {
                        "User requested restart."
                    } else {
                        reason
                    },
                ) {
                    self.append_error(format!("failed to mark task failed: {err}"));
                    return;
                }
            }
        }

        match coordination::restart_failed_tasks(&run_id) {
            Ok(snapshot) => {
                self.snapshot = Some(snapshot);
                self.load_selected_run();
                self.refresh_runs();
                self.restart_dialog.open = false;
                self.restart_dialog.assigned_provider.clear();
            }
            Err(err) => self.append_error(format!("failed to restart failed tasks: {err}")),
        }
    }

    pub(super) fn render_restart_dialog(&mut self, ui: &mut egui::Ui, default_provider: &str) {
        if !self.restart_dialog.open {
            return;
        }

        let tasks = self.restart_dialog_tasks();
        let selected = tasks
            .iter()
            .find(|(task_id, _, _, _)| *task_id == self.restart_dialog.selected_task_id)
            .cloned();
        egui::Window::new("Restart Failed Tasks")
            .collapsible(false)
            .resizable(false)
            .show(ui.ctx(), |ui| {
                if tasks.is_empty() {
                    ui.label("No completed, failed, or canceled tasks are available.");
                } else {
                    egui::ComboBox::from_id_source("restart_task_picker")
                        .selected_text(
                            selected
                                .as_ref()
                                .map(|(_, title, status, _)| format!("{title} [{status:?}]"))
                                .unwrap_or_else(|| "Select a task".to_string()),
                        )
                        .show_ui(ui, |ui| {
                            for (task_id, title, status, _) in &tasks {
                                ui.selectable_value(
                                    &mut self.restart_dialog.selected_task_id,
                                    task_id.clone(),
                                    format!("{title} [{status:?}]"),
                                );
                            }
                        });
                    if let Some((_, _, status, provider)) = selected {
                        ui.label(format!(
                            "Current Model: {}",
                            provider.unwrap_or_else(|| default_provider.to_string())
                        ));
                        if status == CoordinationTaskStatus::Completed {
                            ui.checkbox(
                                &mut self.restart_dialog.mark_selected_task_failed,
                                "Mark selected completed task failed before restart",
                            );
                        } else {
                            self.restart_dialog.mark_selected_task_failed = false;
                            ui.label("Selected task is already failed or canceled.");
                        }
                    }
                    ui.label("Assigned Model Override");
                    egui::ComboBox::from_id_source("restart_task_provider")
                        .selected_text(if self.restart_dialog.assigned_provider.is_empty() {
                            "Keep existing".to_string()
                        } else {
                            self.restart_dialog.assigned_provider.clone()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.restart_dialog.assigned_provider,
                                String::new(),
                                "Keep existing",
                            );
                            for provider in ModelProvider::ALL {
                                ui.selectable_value(
                                    &mut self.restart_dialog.assigned_provider,
                                    provider.as_str().to_string(),
                                    provider.display_name(),
                                );
                            }
                        });
                    ui.label("Reason");
                    ui.text_edit_multiline(&mut self.restart_dialog.reason);
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        self.submit_restart_dialog();
                    }
                    if ui.button("Cancel").clicked() {
                        self.restart_dialog.open = false;
                    }
                });
            });
    }
}
