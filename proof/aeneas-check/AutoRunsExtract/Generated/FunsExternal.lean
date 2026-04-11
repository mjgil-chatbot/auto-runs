import Aeneas
import AutoRunsExtract.Generated.Types

open Aeneas Aeneas.Std Result ControlFlow Error
open AutoRunsExtract

set_option linter.dupNamespace false
set_option linter.hashCommand false
set_option linter.unusedVariables false
set_option maxHeartbeats 1000000

/-- [core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field2_finish] -/
@[rust_fun "core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field2_finish"]
axiom core.fmt.Formatter.debug_struct_field2_finish
  :
  core.fmt.Formatter → Str → Str → Dyn (fun _dyn => core.fmt.Debug _dyn)
    → Str → Dyn (fun _dyn => core.fmt.Debug _dyn) → Result
    ((core.result.Result Unit core.fmt.Error) × core.fmt.Formatter)

/-- [core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field3_finish] -/
@[rust_fun "core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field3_finish"]
axiom core.fmt.Formatter.debug_struct_field3_finish
  :
  core.fmt.Formatter → Str → Str → Dyn (fun _dyn => core.fmt.Debug _dyn)
    → Str → Dyn (fun _dyn => core.fmt.Debug _dyn) → Str → Dyn (fun _dyn
    => core.fmt.Debug _dyn) → Result ((core.result.Result Unit
    core.fmt.Error) × core.fmt.Formatter)

/-- [core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field5_finish] -/
@[rust_fun "core::fmt::{core::fmt::Formatter<'a>}::debug_struct_field5_finish"]
axiom core.fmt.Formatter.debug_struct_field5_finish
  :
  core.fmt.Formatter → Str → Str → Dyn (fun _dyn => core.fmt.Debug _dyn)
    → Str → Dyn (fun _dyn => core.fmt.Debug _dyn) → Str → Dyn (fun _dyn
    => core.fmt.Debug _dyn) → Str → Dyn (fun _dyn => core.fmt.Debug _dyn)
    → Str → Dyn (fun _dyn => core.fmt.Debug _dyn) → Result
    ((core.result.Result Unit core.fmt.Error) × core.fmt.Formatter)

/-- [core::option::{core::fmt::Debug for core::option::Option<T>}::fmt] -/
@[rust_fun "core::option::{core::fmt::Debug<core::option::Option<@T>>}::fmt"]
axiom core.option.Option.Insts.CoreFmtDebug.fmt
  {T : Type} (fmtDebugInst : core.fmt.Debug T) :
  Option T → core.fmt.Formatter → Result ((core.result.Result Unit
    core.fmt.Error) × core.fmt.Formatter)

/-- [core::option::{core::cmp::PartialEq<core::option::Option<T>> for core::option::Option<T>}::eq] -/
@[rust_fun
  "core::option::{core::cmp::PartialEq<core::option::Option<@T>, core::option::Option<@T>>}::eq"]
def core.option.Option.Insts.CoreCmpPartialEqOption.eq
  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :
  Option T → Option T → Result Bool
  | none, none => ok true
  | some x, some y => cmpPartialEqInst.eq x y
  | _, _ => ok false

/-- [core::option::{core::cmp::PartialEq<core::option::Option<T>> for core::option::Option<T>}::ne] -/
@[rust_fun
  "core::option::{core::cmp::PartialEq<core::option::Option<@T>, core::option::Option<@T>>}::ne"]
def core.option.Option.Insts.CoreCmpPartialEqOption.ne
  {T : Type} (cmpPartialEqInst : core.cmp.PartialEq T T) :
  Option T → Option T → Result Bool
  | left, right => do
      let b ← core.option.Option.Insts.CoreCmpPartialEqOption.eq cmpPartialEqInst left right
      ok (¬ b)

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::TaskStatus> for auto_runs_aeneas_model::TaskStatus}::ne] -/
def TaskStatus.Insts.CoreCmpPartialEqTaskStatus.ne
  : TaskStatus → TaskStatus → Result Bool
  | TaskStatus.Queued, TaskStatus.Queued => ok false
  | TaskStatus.Claiming, TaskStatus.Claiming => ok false
  | TaskStatus.Running, TaskStatus.Running => ok false
  | TaskStatus.Failed, TaskStatus.Failed => ok false
  | _, _ => ok true

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::Task> for auto_runs_aeneas_model::Task}::ne] -/
axiom Task.Insts.CoreCmpPartialEqTask.ne : Task → Task → Result Bool

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::Claim> for auto_runs_aeneas_model::Claim}::ne] -/
axiom Claim.Insts.CoreCmpPartialEqClaim.ne : Claim → Claim → Result Bool

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::Driver> for auto_runs_aeneas_model::Driver}::ne] -/
axiom Driver.Insts.CoreCmpPartialEqDriver.ne
  : Driver → Driver → Result Bool

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::RecoveryAction> for auto_runs_aeneas_model::RecoveryAction}::ne] -/
axiom RecoveryAction.Insts.CoreCmpPartialEqRecoveryAction.ne
  : RecoveryAction → RecoveryAction → Result Bool

/-- [auto_runs_aeneas_model::{core::cmp::PartialEq<auto_runs_aeneas_model::State> for auto_runs_aeneas_model::State}::ne] -/
axiom State.Insts.CoreCmpPartialEqState.ne : State → State → Result Bool
