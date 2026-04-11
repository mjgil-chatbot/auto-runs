import Aeneas
import AutoRunsExtract.Generated.Funs

open Aeneas Aeneas.Std Result ControlFlow Error

namespace AutoRunsExtract

def actionIsMissingClaimRequeued (state : State) : Bool :=
  match state.action with
  | RecoveryAction.TaskClaimMissingRequeued => true
  | _ => false

def actionIsMissingClaimFailed (state : State) : Bool :=
  match state.action with
  | RecoveryAction.TaskClaimMissingFailed => true
  | _ => false

def actionIsClaimOrphaned (state : State) : Bool :=
  match state.action with
  | RecoveryAction.TaskClaimOrphaned => true
  | _ => false

def actionIsClaimReconciled (state : State) : Bool :=
  match state.action with
  | RecoveryAction.TaskClaimReconciled => true
  | _ => false

def actionIsDriverStopped (state : State) : Bool :=
  match state.action with
  | RecoveryAction.DriverStoppedStaleExecutable => true
  | _ => false

def actionIsClaimed (state : State) : Bool :=
  match state.action with
  | RecoveryAction.TaskClaimedWithDriverContext => true
  | _ => false

def statusIsQueued (state : State) : Bool :=
  match state.task.status with
  | TaskStatus.Queued => true
  | _ => false

def statusIsFailed (state : State) : Bool :=
  match state.task.status with
  | TaskStatus.Failed => true
  | _ => false

def resultIsTrue (result : Result Bool) : Bool :=
  match result with
  | ok value => value
  | fail _ => false
  | div => false

example :
  resultIsTrue
      (do
        let s ← stabilize (← missing_claim_session)
        let inv ← invariant_holds s
        ok (inv && statusIsQueued s && s.claim_missing_detected && actionIsMissingClaimRequeued s))
    = true := by
  native_decide

example :
  resultIsTrue
      (do
        let s ← stabilize (← exhausted_missing_claim_session)
        let inv ← invariant_holds s
        ok (inv && statusIsFailed s && s.claim_missing_detected && actionIsMissingClaimFailed s))
    = true := by
  native_decide

example :
  resultIsTrue
      (do
        let s ← refresh_claim_state (← orphan_claim_session)
        let has ← task_has_claim s
        ok (has && actionIsClaimOrphaned s && s.claim.isNone))
    = true := by
  native_decide

example :
  resultIsTrue
      (do
        let s ← refresh_claim_state (← drifted_claim_session)
        ok
          (actionIsClaimReconciled s &&
            decide (s.task.claimed_by = some 1775#u32) &&
            decide (s.task.claim_token = some 1775#u32)))
    = true := by
  native_decide

example :
  resultIsTrue
      (do
        let s ← stabilize (← stale_driver_session)
        let inv ← invariant_holds s
        ok (inv && actionIsDriverStopped s && decide (¬ s.driver.active)))
    = true := by
  native_decide

example :
  resultIsTrue
      (do
        let s ← claim_task (← claimable_session)
        let has ← task_has_claim s
        ok
          (has &&
            actionIsClaimed s &&
            decide (s.driver_context_pid = some 1775#u32) &&
            decide (s.driver_context_deleted = false)))
    = true := by
  native_decide

end AutoRunsExtract
