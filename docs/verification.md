# Verification Artifact: Test-to-Behavior Mapping

**Status:** Active — updated as coverage slices land
**Date:** 2026-04-22

---

## Test Topology

Tests use an include-based aggregation pattern. There are no monolithic
`partN.rs` files; each scenario file is named after the behavior it covers.

### Top-Level Integration Tests (`tests/`)

| File | Behaviors Verified |
|------|--------------------|
| `workflow_contracts.rs` | Workflow registry core workflow presence, unique names, required fields, context-file existence, code-review profile presence |

### Coordination Tests (`src/coordination/tests.rs`)

| Include File | Behaviors Verified |
|--------------|--------------------|
| `harness.rs` | `TestCoordinationHarness` builder, fixture helpers |
| `execution_reports_and_restarts.rs` | Task execution reporting, restart logic |
| `planning_and_code_review.rs` | Plan materialization, code-review seeding |
| `review_schema_and_council_seed.rs` | Review schema validation, council seed construction |
| `regressions.rs` | Regression guards for previously fixed bugs |
| `claims_storage_and_restarts.rs` | Claim persistence, lifecycle diagnostics, storage round-trips, restart recovery |
| `provider_cooldowns_and_expansion.rs` | Provider cooldown enforcement, task expansion |
| `task_artifact_validation.rs` | Artifact validation, write-scope enforcement |
| `run_state_and_claim_ownership.rs` | Run state transitions, claim ownership checks |
| `provider_metadata_and_fallback_queries.rs` | Provider metadata, fallback query construction |
| `storage_index_rebuild.rs` | SQLite index rebuild from filesystem state |
| `launch_failures_and_task_categories.rs` | Launch failure handling, task category routing |
| `driver_consistency_and_claim_recovery.rs` | Driver identity breadcrumbs, missing/orphaned claim repair |
| `code_review_document_validation.rs` | Code-review document structure validation |

### Council Tests (`src/coordination/council/tests.rs`)

| Include File | Behaviors Verified |
|--------------|--------------------|
| `tests_selection_and_parsing.rs` | Speaker selection, vote parsing, justification extraction |
| `tests_stall_and_claim_validation.rs` | Stall detection thresholds, claim validation in council |
| `tests_persistence_and_entrypoints.rs` | Council state persistence, entrypoint dispatch |

### Scenario Files Included Indirectly (under `src/coordination/tests/`)

These files are pulled in through secondary include shims such as
`claims_storage_and_restarts.rs`:

| File | Behaviors Verified |
|------|--------------------|
| `claims_and_scope_conflicts.rs` | Claim conflict detection, scope overlap resolution |
| `completion_failures_and_manual_restart.rs` | Task completion failures, manual restart paths |
| `provider_contracts_and_run_config.rs` | Provider contract validation, run configuration |
| `provider_cooldowns_and_iterations.rs` | Provider cooldown iteration logic |
| `review_and_plan_materialization.rs` | Review-to-plan materialization pipeline |
| `storage_snapshots_and_builtins.rs` | Storage snapshots, builtin template validation |
| `code_review_plan_expansion.rs` | Code-review plan expansion logic |

---

## Lean Proofs

The repo now carries a local Lean 4 proof package under `proof/` for the
driver/claim consistency regressions identified in the task/claim consistency regressions.

| Proof File | Invariants Proved |
|------------|-------------------|
| `AutoRunsProof/Model.lean` | Minimal driver/task/claim state machine for consistency repair |
| `AutoRunsProof/Proofs.lean` | Missing-claim repair preserves task/claim consistency, orphan-claim cleanup preserves consistency, deleted-driver rotation deactivates stale drivers, claim-time driver provenance is recorded |
| `aeneas-model/src/lib.rs` | Extraction-friendly Rust model of missing-claim repair, orphan cleanup, claim reconciliation, stale-driver shutdown, and claim-time driver context |
| `aeneas-check/AutoRunsExtract/Generated/Funs.lean` | Aeneas-generated Lean translation of the Rust model |
| `aeneas-check/AutoRunsExtract/Checks.lean` | Lean checks over generated code for requeue, fail, orphan cleanup, claim reconciliation, stale-driver shutdown, and claim-time driver context |

---

## Validation Contract

Every coverage slice must:

1. Run `cargo llvm-cov --release --lib --html` and verify coverage increased
2. Run `cargo test --release --lib` and confirm all tests pass
3. Add entries to this document mapping new test functions to behaviors
4. Run `just proof-check` and `just proof-aeneas-check` when coordination consistency invariants change
5. Not introduce test flakiness (run 3x locally before merging)

## Engineering Gate Coverage

The repo-local hook path now runs a staged-file gate from `.githooks/pre-commit`.

| Behavior | Named checks |
|------|--------------------|
| Gate registry and hook wiring stay parseable and repo-scoped | `tests/test_engineering_gate.py::EngineeringGateTests::test_shell_scripts_parse`, `tests/test_engineering_gate.py::EngineeringGateTests::test_registry_declares_repo_local_contracts`, `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_rejects_registry_for_different_repo_root` |
| CI wiring reuses the same gate entrypoint with a diff range | `tests/test_engineering_gate.py::EngineeringGateTests::test_ci_workflow_runs_engineering_gate_with_diff_base`, `.github/workflows/engineering-gate.yml` |
| Coordination gate wrapper stays isolated from unrelated target-dir contention | `tests/test_engineering_gate.py::EngineeringGateTests::test_coordination_wrapper_uses_isolated_target_dir` |
| Named command groups and diff-range selection stay available for remote/path-sensitive enforcement | `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_can_run_named_command_group`, `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_uses_diff_range_file_discovery` |
| Workflow and coordination code changes require staged verification docs | `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_fails_when_coordination_changes_without_docs`, `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_passes_when_workflow_and_docs_change_together`, `tests/test_engineering_gate.py::EngineeringGateTests::test_gate_uses_real_staged_file_discovery` |

## Portable release regressions

| Test | Behavior |
|------|----------|
| `portable_scaffold_defaults_follow_the_current_home` | Default scaffolds follow the current user's home. |
| `portable_scaffold_explicit_overrides_take_precedence` | Explicit scaffold roots override the defaults. |
| `status_projection_ignores_non_lightweight_workflows_without_creating_snapshots` | Lightweight status refresh does not mutate other workflows. |
| `provider_unavailable_failure_does_not_schedule_repository_self_heal` | Provider outages do not trigger recursive repository repair. |
| `publish_tracks_its_base_preserving_head_index_and_workspace` | Sequential local publication preserves unrelated workspace state. |
| `publish_preserves_a_divergent_local_branch_head` | Publication never advances a divergent local branch. |
| `publish_rejects_external_conflict_after_unrelated_local_publish` | Publishing another file cannot authorize overwriting an external source conflict. |

The existing workflow integration tests also check that every registry context
file exists and that the repository review profile is present. The registry
uses current architecture documents instead of unavailable historical paths.

Release verification runs all available library, binary, and integration tests
serially with all Cargo features. The independent contracts crate and extracted
Rust model have separate locked release checks, builds, and tests. Native Rust
checks do not establish live-provider, container, desktop, or Lean proof behavior.
