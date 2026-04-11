#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskStatus {
    Queued,
    Claiming,
    Running,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Task {
    pub status: TaskStatus,
    pub claimed_by: Option<u32>,
    pub claim_token: Option<u32>,
    pub attempt_count: u32,
    pub max_attempts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    pub owner_pid: u32,
    pub claim_token: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Driver {
    pub pid: u32,
    pub active: bool,
    pub executable_deleted: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    None,
    TaskClaimReconciled,
    TaskClaimMissingRequeued,
    TaskClaimMissingFailed,
    TaskClaimOrphaned,
    DriverStoppedStaleExecutable,
    TaskClaimedWithDriverContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub task: Task,
    pub claim: Option<Claim>,
    pub driver: Driver,
    pub action: RecoveryAction,
    pub claim_missing_detected: bool,
    pub driver_context_pid: Option<u32>,
    pub driver_context_deleted: bool,
}

pub fn stale_claim_recovery_status(_task: Task) -> TaskStatus {
    TaskStatus::Queued
}

pub fn task_is_active(status: TaskStatus) -> bool {
    status == TaskStatus::Claiming || status == TaskStatus::Running
}

pub fn active_task_claim_drifted(task: Task, claim: Claim) -> bool {
    task.claimed_by != Some(claim.owner_pid) || task.claim_token != Some(claim.claim_token)
}

pub fn task_has_claim(state: State) -> bool {
    if task_is_active(state.task.status) {
        state.claim.is_some()
    } else {
        true
    }
}

pub fn deleted_driver_inactive(state: State) -> bool {
    !state.driver.executable_deleted || !state.driver.active
}

pub fn invariant_holds(state: State) -> bool {
    task_has_claim(state) && deleted_driver_inactive(state)
}

pub fn requeue_task_after_stale_claim(task: Task) -> Task {
    Task {
        status: stale_claim_recovery_status(task),
        claimed_by: None,
        claim_token: None,
        attempt_count: task.attempt_count,
        max_attempts: task.max_attempts,
    }
}

pub fn reconcile_active_task_and_claim(state: State) -> State {
    if !task_is_active(state.task.status) {
        return state;
    }

    match state.claim {
        Some(claim) => {
            if active_task_claim_drifted(state.task, claim) {
                State {
                    task: Task {
                        status: state.task.status,
                        claimed_by: Some(claim.owner_pid),
                        claim_token: Some(claim.claim_token),
                        attempt_count: state.task.attempt_count,
                        max_attempts: state.task.max_attempts,
                    },
                    claim: state.claim,
                    driver: state.driver,
                    action: RecoveryAction::TaskClaimReconciled,
                    claim_missing_detected: false,
                    driver_context_pid: state.driver_context_pid,
                    driver_context_deleted: state.driver_context_deleted,
                }
            } else {
                state
            }
        }
        None => {
            let repaired_task = requeue_task_after_stale_claim(state.task);
            State {
                task: repaired_task,
                claim: None,
                driver: state.driver,
                action: RecoveryAction::TaskClaimMissingRequeued,
                claim_missing_detected: true,
                driver_context_pid: state.driver_context_pid,
                driver_context_deleted: state.driver_context_deleted,
            }
        }
    }
}

pub fn drop_orphan_claim(state: State) -> State {
    if task_is_active(state.task.status) || state.claim.is_none() {
        return state;
    }

    State {
        task: state.task,
        claim: None,
        driver: state.driver,
        action: RecoveryAction::TaskClaimOrphaned,
        claim_missing_detected: state.claim_missing_detected,
        driver_context_pid: state.driver_context_pid,
        driver_context_deleted: state.driver_context_deleted,
    }
}

pub fn stop_deleted_driver(state: State) -> State {
    if !state.driver.active || !state.driver.executable_deleted {
        return state;
    }

    State {
        task: state.task,
        claim: state.claim,
        driver: Driver {
            pid: state.driver.pid,
            active: false,
            executable_deleted: true,
        },
        action: RecoveryAction::DriverStoppedStaleExecutable,
        claim_missing_detected: state.claim_missing_detected,
        driver_context_pid: state.driver_context_pid,
        driver_context_deleted: state.driver_context_deleted,
    }
}

pub fn claim_task(state: State) -> State {
    if !state.driver.active || state.driver.executable_deleted || state.task.status != TaskStatus::Queued {
        return state;
    }

    let claim = Claim {
        owner_pid: state.driver.pid,
        claim_token: state.driver.pid,
    };
    State {
        task: Task {
            status: TaskStatus::Running,
            claimed_by: Some(claim.owner_pid),
            claim_token: Some(claim.claim_token),
            attempt_count: state.task.attempt_count,
            max_attempts: state.task.max_attempts,
        },
        claim: Some(claim),
        driver: state.driver,
        action: RecoveryAction::TaskClaimedWithDriverContext,
        claim_missing_detected: false,
        driver_context_pid: Some(state.driver.pid),
        driver_context_deleted: state.driver.executable_deleted,
    }
}

pub fn refresh_claim_state(state: State) -> State {
    drop_orphan_claim(reconcile_active_task_and_claim(state))
}

pub fn stabilize(state: State) -> State {
    stop_deleted_driver(refresh_claim_state(state))
}

pub fn missing_claim_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Running,
            claimed_by: Some(1775),
            claim_token: Some(1775),
            attempt_count: 1,
            max_attempts: 3,
        },
        claim: None,
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: false,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}

pub fn stale_driver_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Queued,
            claimed_by: None,
            claim_token: None,
            attempt_count: 0,
            max_attempts: 3,
        },
        claim: None,
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: true,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}

pub fn orphan_claim_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Queued,
            claimed_by: None,
            claim_token: None,
            attempt_count: 1,
            max_attempts: 3,
        },
        claim: Some(Claim {
            owner_pid: 1775,
            claim_token: 1775,
        }),
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: false,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}

pub fn exhausted_missing_claim_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Running,
            claimed_by: Some(1775),
            claim_token: Some(1775),
            attempt_count: 1,
            max_attempts: 1,
        },
        claim: None,
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: false,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}

pub fn drifted_claim_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Running,
            claimed_by: Some(4000),
            claim_token: Some(4000),
            attempt_count: 1,
            max_attempts: 3,
        },
        claim: Some(Claim {
            owner_pid: 1775,
            claim_token: 1775,
        }),
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: false,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}

pub fn claimable_session() -> State {
    State {
        task: Task {
            status: TaskStatus::Queued,
            claimed_by: None,
            claim_token: None,
            attempt_count: 0,
            max_attempts: 3,
        },
        claim: None,
        driver: Driver {
            pid: 1775,
            active: true,
            executable_deleted: false,
        },
        action: RecoveryAction::None,
        claim_missing_detected: false,
        driver_context_pid: None,
        driver_context_deleted: false,
    }
}
