use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use auto_runs::coordination::{
    self, CoordinationEvent, CoordinationInterrupt, CoordinationRunSnapshot, InterruptResolution,
    RunSummaryRow, UpsertRunInput,
};

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
    pub fn spawn(poll_interval: Duration) -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (update_tx, update_rx) = mpsc::channel();
        thread::spawn(move || background_loop(command_rx, update_tx, poll_interval));
        Self {
            command_tx,
            update_rx,
        }
    }

    pub fn send(&self, command: UiCommand) -> Result<(), String> {
        self.command_tx
            .send(command)
            .map_err(|_| "background UI runtime is unavailable".to_string())
    }

    pub fn try_recv(&self) -> Result<UiUpdate, mpsc::TryRecvError> {
        self.update_rx.try_recv()
    }
}

fn background_loop(
    command_rx: Receiver<UiCommand>,
    update_tx: Sender<UiUpdate>,
    poll_interval: Duration,
) {
    let mut state = WorkerState::default();
    if send_refresh(&mut state, &update_tx).is_err() {
        return;
    }

    loop {
        let should_refresh = match command_rx.recv_timeout(poll_interval) {
            Ok(command) => {
                if apply_command(command, &mut state, &update_tx).is_err() {
                    break;
                }
                while let Ok(command) = command_rx.try_recv() {
                    if apply_command(command, &mut state, &update_tx).is_err() {
                        return;
                    }
                }
                true
            }
            Err(mpsc::RecvTimeoutError::Timeout) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        if should_refresh && send_refresh(&mut state, &update_tx).is_err() {
            break;
        }
    }
}

fn apply_command(
    command: UiCommand,
    state: &mut WorkerState,
    update_tx: &Sender<UiUpdate>,
) -> Result<(), ()> {
    let result = match command {
        UiCommand::SetSelectedRun(run_id) => {
            state.selected_run_id = run_id;
            Ok(())
        }
        UiCommand::RefreshNow => Ok(()),
        UiCommand::CreateRun(input) => match coordination::upsert_run(input) {
            Ok(snapshot) => {
                state.selected_run_id = Some(snapshot.run.run_id);
                Ok(())
            }
            Err(err) => Err(format!("failed to create run: {err}")),
        },
        UiCommand::RestartFailed { run_id } => match coordination::restart_failed_tasks(&run_id) {
            Ok(_) => {
                state.selected_run_id = Some(run_id);
                Ok(())
            }
            Err(err) => Err(format!("failed to restart failed tasks: {err}")),
        },
        UiCommand::StopRun { run_id, reason } => match coordination::stop_run(&run_id, &reason) {
            Ok(_) => {
                state.selected_run_id = Some(run_id);
                Ok(())
            }
            Err(err) => Err(format!("failed to stop run: {err}")),
        },
        UiCommand::LoadInterrupts { run_id } => {
            state.selected_run_id = Some(run_id);
            Ok(())
        }
        UiCommand::ResolveInterrupt {
            run_id,
            interrupt_id,
            resolution,
        } => match coordination::resolve_interrupt(&run_id, &interrupt_id, resolution) {
            Ok(_) => {
                state.selected_run_id = Some(run_id);
                Ok(())
            }
            Err(err) => Err(format!("failed to resolve interrupt: {err}")),
        },
    };

    if let Err(message) = result {
        send_error(update_tx, message)?;
    }
    Ok(())
}

fn send_refresh(state: &mut WorkerState, update_tx: &Sender<UiUpdate>) -> Result<(), ()> {
    match collect_state(state) {
        Ok(snapshot) => update_tx.send(UiUpdate::State(snapshot)).map_err(|_| ())?,
        Err(err) => send_error(update_tx, err.to_string())?,
    }
    Ok(())
}

fn send_error(update_tx: &Sender<UiUpdate>, message: String) -> Result<(), ()> {
    update_tx.send(UiUpdate::Error(message)).map_err(|_| ())
}

fn collect_state(state: &mut WorkerState) -> anyhow::Result<UiState> {
    let runs = coordination::query_runs_filtered(None, None, None, Some(200), None)?;
    if state
        .selected_run_id
        .as_ref()
        .is_some_and(|selected| !runs.iter().any(|run| run.run_id == *selected))
    {
        state.selected_run_id = None;
    }

    let mut snapshot = None;
    let mut events = Vec::new();
    let mut interrupts = Vec::new();
    if let Some(run_id) = state.selected_run_id.clone() {
        snapshot = Some(coordination::load_run_snapshot(&run_id)?);
        events = coordination::load_events(&run_id, Some(40), None)?;
        interrupts = coordination::load_interrupts(&run_id);
    }

    Ok(UiState {
        runs,
        selected_run_id: state.selected_run_id.clone(),
        snapshot,
        events,
        interrupts,
    })
}
