# Proof

This directory contains a small Lean 4 proof project for the coordination
consistency regressions identified from the task/claim consistency regressions.

## Scope

The model is intentionally narrow. It formalizes the control-plane repair rules
behind three concrete runtime protections:

- active tasks must not stay `running` or `claiming` after their claim file is
  gone
- orphaned claim files must not outlive the durable task state that should own
  them
- a deleted background-driver executable must stop claiming work, and every new
  claim must record the claiming driver identity

## What Is Proved

The proof package exports machine-checked theorems for:

- missing-claim repair re-establishes the task/claim invariant
- orphan-claim cleanup preserves that invariant
- deleted-driver rotation deactivates stale drivers
- claim-time provenance records the claiming driver's PID and executable path

It also includes concrete executable examples for the two regression
failures:

- a `running` task with no claim stabilizes back to `queued`
- a driver whose executable is marked deleted stabilizes to `active = false`

## Layout

- `lakefile.lean`: Lean project definition
- `AutoRunsProof/Model.lean`: minimal task/claim/driver state machine
- `AutoRunsProof/Proofs.lean`: machine-checked invariants and examples
- `aeneas-model/`: extraction-friendly Rust model of the same repair branches
- `aeneas-check/`: Aeneas-generated Lean code plus regression checks over the generated functions

## Verification

Build the proof project with:

```bash
just proof-check
```

Regenerate the extracted Lean code and build the Aeneas-backed check package with:

```bash
just proof-aeneas-check
```
