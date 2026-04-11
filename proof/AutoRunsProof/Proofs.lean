import AutoRunsProof.Model

namespace AutoRunsProof

theorem requeueOrFailTask_clears_claimedBy (task : Task) :
    (requeueOrFailTask task).claimedBy = none := by
  unfold requeueOrFailTask
  by_cases h : task.attemptCount >= task.maxAttempts <;> simp [h]

theorem repaired_task_without_claim_is_safe
    (task : Task)
    (driver : Driver)
    (events : List Event) :
    taskHasClaim
      { task := requeueOrFailTask task
        claim := none
        driver := driver
        events := events
      } = true := by
  unfold taskHasClaim requeueOrFailTask
  by_cases h : task.attemptCount >= task.maxAttempts <;> simp [h]

theorem reconcileMissingClaim_establishes_taskHasClaim_for_components
    (status : TaskStatus)
    (claimedBy : Option Nat)
    (attemptCount maxAttempts : Nat)
    (claim : Option Claim)
    (driver : Driver)
    (events : List Event) :
    taskHasClaim
      (reconcileMissingClaim
        { task := { status := status, claimedBy := claimedBy, attemptCount := attemptCount, maxAttempts := maxAttempts }
          claim := claim
          driver := driver
          events := events
        }) = true := by
  cases status <;> cases claim <;> by_cases hAttempt : attemptCount >= maxAttempts <;>
    simp [reconcileMissingClaim, taskHasClaim, requeueOrFailTask, hAttempt]

theorem reconcileMissingClaim_establishes_taskHasClaim (state : State) :
    taskHasClaim (reconcileMissingClaim state) = true := by
  cases state with
  | mk task claim driver events =>
      cases task with
      | mk status claimedBy attemptCount maxAttempts =>
          exact reconcileMissingClaim_establishes_taskHasClaim_for_components
            status
            claimedBy
            attemptCount
            maxAttempts
            claim
            driver
            events

theorem rotateDeletedDriver_establishes_deletedDriverInactive (state : State) :
    deletedDriverInactive (rotateDeletedDriver state) = true := by
  unfold rotateDeletedDriver deletedDriverInactive
  by_cases h : state.driver.executableDeleted <;> simp [h]

theorem stabilize_establishes_invariant_for_components
    (status : TaskStatus)
    (claimedBy : Option Nat)
    (attemptCount maxAttempts : Nat)
    (claim : Option Claim)
    (driver : Driver)
    (events : List Event) :
    invariant
      (stabilize
        { task := { status := status, claimedBy := claimedBy, attemptCount := attemptCount, maxAttempts := maxAttempts }
          claim := claim
          driver := driver
          events := events
        }) := by
  cases status <;> cases claim <;> by_cases hDeleted : driver.executableDeleted <;>
    by_cases hAttempt : attemptCount >= maxAttempts <;>
      simp
        [invariant, stabilize, taskHasClaim, deletedDriverInactive,
          reconcileMissingClaim, dropOrphanClaim, rotateDeletedDriver,
          requeueOrFailTask, hDeleted, hAttempt]

theorem stabilize_establishes_invariant (state : State) :
    invariant (stabilize state) := by
  cases state with
  | mk task claim driver events =>
      cases task with
      | mk status claimedBy attemptCount maxAttempts =>
          exact stabilize_establishes_invariant_for_components
            status
            claimedBy
            attemptCount
            maxAttempts
            claim
            driver
            events

theorem dropOrphanClaim_removes_claim_for_terminal_task
    (task : Task)
    (claim : Claim)
    (hQueued : task.status = .queued ∨ task.status = .failed) :
    (dropOrphanClaim
      { task := task
        claim := some claim
        driver := { pid := 1, active := true, executablePath := "auto-runs", executableDeleted := false }
        events := []
      }).claim = none := by
  rcases hQueued with hQueued | hFailed
  · simp [dropOrphanClaim, hQueued]
  · simp [dropOrphanClaim, hFailed]

theorem claimTask_records_driver_identity
    (state : State)
    (hActive : state.driver.active = true)
    (hFresh : state.driver.executableDeleted = false)
    (hQueued : state.task.status = .queued) :
    (claimTask state).events.head? =
      some
        { kind := .driverClaimContext
          driverPid := some state.driver.pid
          executablePath := some state.driver.executablePath
          executableDeleted := false
        } := by
  simp [claimTask, hActive, hFresh, hQueued]

theorem claimTask_establishes_taskHasClaim
    (state : State)
    (hActive : state.driver.active = true)
    (hFresh : state.driver.executableDeleted = false)
    (hQueued : state.task.status = .queued) :
    taskHasClaim (claimTask state) = true := by
  simp [claimTask, taskHasClaim, hActive, hFresh, hQueued]

example : (stabilize missingClaimSession).task.status = .queued := by
  rfl

example : (stabilize staleDriverSession).driver.active = false := by
  rfl

end AutoRunsProof
