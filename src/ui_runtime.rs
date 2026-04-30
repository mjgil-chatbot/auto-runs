use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;
use auto_runs::coordination::CoordinationEvent;
use auto_runs::coordination::CoordinationInterrupt;
use auto_runs::coordination::CoordinationRunSnapshot;
use auto_runs::coordination::InterruptResolution;
use auto_runs::coordination::RunSummaryRow;
use auto_runs::coordination::UpsertRunInput;
pub struct UiRuntime {
    command_tx: Sender<UiCommand>,
    update_rx: Receiver<UiUpdate>,
}
pub struct UiState {
    pub runs: Vec<RunSummaryRow>,
    pub selected_run_id: Option<String>,
    pub snapshot: Option<CoordinationRunSnapshot>,
    pub events: Vec<CoordinationEvent>,
    pub interrupts: Vec<CoordinationInterrupt>,
}
pub enum UiUpdate {
    State(UiState),
    Error(String),
}
pub enum UiCommand {
    SetSelectedRun(Option<String>),
    RefreshNow,
    CreateRun(UpsertRunInput),
    RestartFailed { run_id: String },
    StopRun { run_id: String, reason: String },
    LoadInterrupts { run_id: String },
    ResolveInterrupt {
        run_id: String,
        interrupt_id: String,
        resolution: InterruptResolution,
    },
}
#[derive(Default)]
struct WorkerState {
    selected_run_id: Option<String>,
}
impl UiRuntime {
    pub fn send(&self, command: UiCommand) -> Result<(), String> {
        self.command_tx
            .send(command)
            .map_err(|_| "background UI runtime is unavailable".to_string())
    }}
impl UiRuntime {
    pub fn try_recv(&self) -> Result<UiUpdate, mpsc::TryRecvError> {
        self.update_rx.try_recv()
    }}
fn send_error(update_tx: &Sender<UiUpdate>, message: String) -> Result<(), ()> {
    update_tx.send(UiUpdate::Error(message)).map_err(|_| ())
}
