namespace AutoRunsProof

inductive TaskStatus where
  | queued
  | claiming
  | running
  | failed
  deriving DecidableEq, Repr

structure Task where
  status : TaskStatus
  claimedBy : Option Nat
  attemptCount : Nat
  maxAttempts : Nat
  deriving DecidableEq, Repr

structure Claim where
  ownerPid : Nat
  deriving DecidableEq, Repr

structure Driver where
  pid : Nat
  active : Bool
  executablePath : String
  executableDeleted : Bool
  deriving DecidableEq, Repr

inductive EventKind where
  | taskClaimMissing
  | taskRequeued
  | taskFailed
  | taskClaimOrphaned
  | driverClaimContext
  deriving DecidableEq, Repr

structure Event where
  kind : EventKind
  driverPid : Option Nat := none
  executablePath : Option String := none
  executableDeleted : Bool := false
  deriving DecidableEq, Repr

structure State where
  task : Task
  claim : Option Claim
  driver : Driver
  events : List Event
  deriving Repr

def taskHasClaim (state : State) : Bool :=
  match state.task.status, state.claim with
  | .claiming, some _ => true
  | .running, some _ => true
  | .claiming, none => false
  | .running, none => false
  | _, _ => true

def deletedDriverInactive (state : State) : Bool :=
  !state.driver.executableDeleted || !state.driver.active

def invariant (state : State) : Prop :=
  taskHasClaim state = true ∧ deletedDriverInactive state = true

def requeueOrFailTask (task : Task) : Task :=
  if task.attemptCount >= task.maxAttempts then
    { task with status := .failed, claimedBy := none }
  else
    { task with status := .queued, claimedBy := none }

def reconcileMissingClaim (state : State) : State :=
  match state.task.status, state.claim with
  | .claiming, none =>
      let repairedTask := requeueOrFailTask state.task
      let recoveryEvent :=
        if repairedTask.status == .failed then
          [{ kind := .taskFailed }]
        else
          [{ kind := .taskRequeued }]
      { state with
          task := repairedTask
          events := ({ kind := .taskClaimMissing } :: recoveryEvent) ++ state.events
      }
  | .running, none =>
      let repairedTask := requeueOrFailTask state.task
      let recoveryEvent :=
        if repairedTask.status == .failed then
          [{ kind := .taskFailed }]
        else
          [{ kind := .taskRequeued }]
      { state with
          task := repairedTask
          events := ({ kind := .taskClaimMissing } :: recoveryEvent) ++ state.events
      }
  | _, _ => state

def dropOrphanClaim (state : State) : State :=
  match state.task.status, state.claim with
  | .queued, some _ =>
      { state with
          claim := none
          events := { kind := .taskClaimOrphaned } :: state.events
      }
  | .failed, some _ =>
      { state with
          claim := none
          events := { kind := .taskClaimOrphaned } :: state.events
      }
  | _, _ => state

def rotateDeletedDriver (state : State) : State :=
  if state.driver.executableDeleted then
    { state with driver := { state.driver with active := false } }
  else
    state

def claimTask (state : State) : State :=
  if state.driver.active && !state.driver.executableDeleted && state.task.status == .queued then
    let event :=
      { kind := .driverClaimContext
        driverPid := some state.driver.pid
        executablePath := some state.driver.executablePath
        executableDeleted := state.driver.executableDeleted
      }
    { state with
        task := { state.task with status := .running, claimedBy := some state.driver.pid }
        claim := some { ownerPid := state.driver.pid }
        events := event :: state.events
    }
  else
    state

def stabilize (state : State) : State :=
  rotateDeletedDriver (dropOrphanClaim (reconcileMissingClaim state))

def missingClaimSession : State :=
  { task := { status := .running, claimedBy := some 1775, attemptCount := 1, maxAttempts := 3 }
    claim := none
    driver := { pid := 1775, active := true, executablePath := "/home/developer/.cargo/bin/auto-runs", executableDeleted := false }
    events := []
  }

def staleDriverSession : State :=
  { task := { status := .queued, claimedBy := none, attemptCount := 0, maxAttempts := 3 }
    claim := none
    driver := { pid := 1775, active := true, executablePath := "/home/developer/.cargo/bin/auto-runs (deleted)", executableDeleted := true }
    events := []
  }

end AutoRunsProof
