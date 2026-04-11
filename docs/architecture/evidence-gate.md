# Evidence Gate

**Status**: Current gate definition
**Last updated**: 2026-05-21

This document defines the repo-local evidence gate for `auto-runs`.

The gate is now wired through a repo-aware contract registry and versioned
hook path:

- `contracts/engineering-gate.toml`
- `.github/workflows/engineering-gate.yml`
- `scripts/verify-engineering-gate.sh`
- `scripts/verify-coordination-gate.sh`
- `.githooks/pre-commit`

## Purpose

The acceptance unit for `auto-runs` should not be a plausible edit. It should
be a bounded evidence package:

```text
{ contract, focused regression, implementation, verification mapping }
```

## Current Contracts

The current registry distinguishes three gate surfaces:

1. `engineering_docs`
   Keeps the docs, hook path, and gate scripts coherent.
2. `workflow_registry`
   Covers workflow registry and context-file changes.
3. `coordination_runtime`
   Covers coordination, council, and code-review runtime changes.

## Current Commands

The gate currently dispatches:

- `python3 -m unittest tests.test_engineering_gate`
- `bash scripts/verify-coordination-gate.sh --workflow`
- `bash scripts/verify-coordination-gate.sh --coordination`

The coordination wrapper pins `CARGO_TARGET_DIR` to a user-scoped
`target/engineering-gate-local-$USER/` path by default so gate-time release
checks do not contend with unrelated cargo work in the default target tree or
reuse root-owned artifacts from containerized runs.

The wrapper no longer hardcodes its release test list. It delegates to named
command groups in `contracts/engineering-gate.toml`, and the GitHub Actions
workflow now calls the same `scripts/verify-engineering-gate.sh` entrypoint
with `--diff-base <sha> --diff-head HEAD` so remote enforcement stays
path-sensitive.

## Failure Handling

If a change fails the gate, the next step should be one of:

- update the owning architecture or verification docs
- add the missing focused regression
- narrow the edit until it fits an existing checked contract

## Related Docs

- `docs/architecture/overview.md`
- `docs/verification.md`
