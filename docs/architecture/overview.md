# Current Architecture

This project is a standalone Rust desktop application for creating, driving,
and inspecting autonomous coordination runs.

The documents under `docs/architecture/` are the current implementation source
of truth. The workflow registry points to these documents for repository context.
Run-generated design artifacts are written separately under `docs/autonomous/`.

## Main Components

- `auto-runs` is the egui desktop UI and the operator surface for new runs.
- `auto_runs::coordination` owns durable run state, task seeding, claims,
  artifacts, and restart/stop operations.
- `auto_runs::workflow` and the workflow registry map workflow modes and
  provider capabilities into concrete run plans.
- `chatbot_lib` is the library from the sibling `chatbot-worker` repo. It
  provides provider execution, worker behavior, hooks, memory integration, and
  shared config.
- `auto-runs-worker` is the detached companion process that executes
  provider-backed coordination tasks once they are claimed.

The `auto_runs` library only re-exports the shared `chatbot_lib` surfaces that
the app and its local binaries still consume directly; split-era blanket
pass-through exports are intentionally trimmed.

## Runtime Model

1. The UI creates a run with workspace, workflow mode, execution environment,
   provider hints, policy knobs, and workflow-specific note-backed fields.
   In the create-run form, `Workflow` now renders immediately below `Goal`,
   and the quick workflow shortcut buttons render immediately below that
   selector before the rest of the shared fields. `Auto-retry failed tasks`
   defaults on for a fresh form and is reset on whenever the operator changes
   `Workflow` through the dropdown or a quick-select button.
2. `coordination::upsert_run` writes durable run metadata and the seeded task
   set under the coordination spool.
3. A separately installed background driver service keeps polling run state
   from disk and calls `drive_run_for_instance(...)` for every nonterminal run
   to advance runnable work, even when the desktop window is closed. Selection
   only controls which run's detail snapshot and event list are loaded into the
   main panel.
4. Claim logic acquires leases, enforces dependencies, and blocks conflicting
   write scopes before launch. It also refuses new claims while an open
   interrupt keeps the run in `WaitingForPolicy`.
5. Claimed tasks are executed either by built-in handlers or by launching
   `auto-runs-worker` for provider-backed work. Provider-backed launches now
   pass through input and dialog rails first, so malformed config can be
   rejected and the assembled provider query can be rewritten before the
   provider call starts.
6. Task artifacts, events, and task snapshots are written back to disk and
   become the source of truth for later stages and later UI refreshes.
   For contract-backed provider tasks, the worker now validates a typed
   artifact envelope in-process and records `task_contract_reask` events when
   it has to re-ask for malformed structured output before finalizing the
   artifact. The same contract lookup now drives real structured-output mode
   selection from the provider capability registry, so schema-capable providers
   receive a native JSON Schema override while the rest stay on prompt-only
   fallback. `GithubBranchFix` planner tasks are the explicit exception: their
   `PlanFlatTasks` responses stay as raw JSON task arrays, so that workflow
   does not emit typed-envelope reasks for its planners. For mutating tasks,
   completion now also runs execution and output rails before sync-back. Those
   rails block out-of-scope file writes, writes without declared edit
   permission, apply-without-review-approval cases, and stub-marker outputs;
   every non-allow decision is recorded as a `rail_decision` event in
   `events.jsonl`.
   Planner-array extraction now also skips incidental inline arrays that may
   appear in surrounding prose (for example `write_scope: ["build.sh"]`) and
   keeps scanning until it finds the actual top-level generated-task JSON
   array.
7. Run summary rows are updated when task lifecycle changes are persisted, so
   list-view status and `updated_at` reflect the real event time rather than a
   later detail-view refresh.
8. Review, apply, retry, and stop actions all operate by mutating the same
   durable run state rather than keeping separate UI-only state.

When every task has reached `Completed`, the coordinator still recomputes a
run-level completion gate from the persisted artifacts before it marks the run
`Completed`. Completed artifacts that still report `blocked`, `partial`, or
`failed` normally keep the run failed, but two kinds of `partial` artifact are
treated as superseded loop state instead of terminal failure: lightweight
review passes that already spawned a follow-up review, and planning tasks that
already handed control to child tasks.

Before built-in or provider-backed task execution begins, the coordinator
ensures the task workspace directory exists. When it has to create that
directory, it appends a `task_workspace_created` ledger event so the UI shows
the workspace creation explicitly in Recent Events.

For `LightweightImplementation` runs, workspace preparation reconciles the task
workspace even when the directory already exists. The current create-run UI
offers `N/A`, `Rust`, or `Zig`; `Rust` recopies the selected scaffold-managed
repo configuration files from `$AUTO_RUNS_RUST_SCAFFOLD_ROOT`, and `Zig`
rewrites the runtime-owned `.gitignore` plus hook files to a built-in Zig
profile. Both modes overwrite drift in those owned files, mark copied scripts
executable, and reconfigure the repo-managed git hooks before task execution.
If the lightweight task-list note was previously stored as an absolute path
under the workspace root, workspace/runtime loading also normalizes it back to
a repo-relative path before prompts and generated scopes consume it. If the
lightweight workspace directory exists but has not been initialized as a git
repo yet, the same prep path initializes it in place before the worker turn
starts.

`GithubBranchFix` now special-cases that startup path: the operator selects the
primary repo checkout as `workspace_path`, `upsert_run` immediately creates or
reuses a run-specific linked git worktree at
`{workspace_path}-{sanitized-branch-name}-{run-id-fragment}`, rewrites the run
to that worktree path, and later recreates the same worktree if a retry finds
it missing. That per-run suffix keeps concurrent branch-fix runs on the same
branch from sharing one mutable workspace. Branch-fix write tasks also recover
from automatic git publish failures differently than generic lightweight tasks:
while attempt budget remains, the same task is requeued with the hook or push
stderr injected back into its prompt so the next worker turn can repair the
publish blocker or resume pushing an already-created local commit.
Its seeded implementation and validation planner tasks also override the
default 10-minute planner estimate to 60 minutes, so the provider timeout path
gives branch-fix planning turns more room before automatic retry.

Stopping a run cancels active work without clearing the persisted
`auto_retry_failed_tasks` preference, so a later "Restart Failed" action
resumes with the same retry policy the run was created with.

Manual restart now creates a fresh execution generation for the restarted
lineage instead of replaying the old artifact surface in place. The failed
task, the upstream dependencies whose evidence it consumes, and every
downstream task affected by those refreshed dependencies are moved onto the
new generation together. Dependency readiness and provider prompt assembly now
only consider completed artifacts from the same generation, which prevents a
restarted branch from inheriting stale blocked summaries or stale review/plan
context from the previous attempt.

For `GithubBranchFix`, a terminal review that ends `partial` without emitting
new follow-up tasks no longer strands the run at the completion gate while CI
is still merely in progress. The coordinator synthesizes a new wait-for-GitHub-
Actions read task and chains another follow-up review, and the completion gate
also ignores superseded `partial` write-task artifacts once later validation
review tasks depend on them. That wait task now also requeues itself when its
artifact reports `WAITING_FOR_SUCCESS:` instead of unblocking the next review
before a successful Actions run has actually landed.

Branch-fix validation reads that confirm a successful post-push Actions run
now use that same runtime gate. The coordinator does not trust a plain-text
`COMPLETED` report by itself; it checks live GitHub run state for the current
`HEAD` and requeues the validation read if no completed successful run exists
yet.

The stop path also terminates provider worker processes that still own or
heartbeat claims for that run. It no longer relies on a best-effort detached
launcher-only signal; claimed worker PIDs are collected from the live claim
files, sent `SIGTERM`, and escalated to `SIGKILL` if they do not exit promptly.

When the operator keeps the default design-doc path, `upsert_run` rewrites it
to `docs/autonomous/<workflow>-doc-<hash>.md` using the workflow slug plus a
short run-ID hash. That keeps default artifacts distinct across runs without
requiring the UI to guess a run ID up front.

For installed `auto-runs` binaries, worker launch falls back from the current
binary directory to the repo-local `target/<profile>` build output. Developer
builds also share a Cargo target dir with the sibling `chatbot-worker` repo so
the shared runtime crate and release artifacts stay co-located.

Run driving is no longer selection-scoped in the desktop UI. A single OS-level
driver service now owns the always-on drive loop, guarded by a single-process
lock under the coordination spool. The UI refresh path only reloads summaries
and detail snapshots from disk. That means stale-claim expiry and dependency
unblocking continue without requiring the UI to stay open, but it also means
background progress depends on the driver service actually being installed and
running.

Provider task ownership is now worker-heartbeat driven instead of short-lease
driven. Detached workers still write durable claim records, but those records
are treated as worker-state ledgers keyed by `last_heartbeat_at`: workers
heartbeat once per minute, the driver/launcher considers a worker stale only
after five minutes without a heartbeat, and the launcher also kills a detached
worker when it runs past the normal task timeout derived from
`estimated_minutes`. Claim-state refresh still repairs missing/orphaned claim
files, but it now defers repair while the recorded owner PID is still alive and
inside that five-minute heartbeat window. The claim path is serialized against
refresh with the same run-scoped lock so refresh cannot delete a newly created
worker-state file from a stale task snapshot.
Dependency-unblock maintenance now writes only the tasks whose status actually
changed instead of rewriting every task file from the loaded snapshot, which
prevents an older snapshot from clobbering a live worker-owned task. Terminal
task completion also clears `claimed_by` before removing the claim file so
completed tasks do not appear queued or still owned after success.
Detached per-task launches use a direct `autonomous <run_id> <task_id> ...`
handoff so the spawned worker adopts the already-claimed task immediately; the
FIFO pull queue remains a separate path for pooled workers rather than for
driver-launched task execution.

Every `task_claimed` event now also carries the claiming driver identity when
the claimer matches the background driver heartbeat file. That embeds the
driver PID, startup time, executable path, and deleted-binary flag directly
into the run ledger so stale-driver ownership is visible from the same event
stream as task retries and stale-claim recovery.

The Runs sidebar keys each visible row by `run_id` before rendering its
interactive widgets. That keeps pointer and focus state pinned to the same run
while the polled summary list refreshes underneath the UI.

The preferred operational path is a user-managed OS service whose entrypoint is
`auto-runs drive-forever`:

- Linux installs `~/.config/systemd/user/auto-runs-driver.service`
- macOS installs `~/Library/LaunchAgents/com.mjgil.auto-runs-driver.plist`

Service lifecycle is managed by repo shell scripts rather than UI code or
extra Rust CLI subcommands. The normal launch scripts ensure that service is
installed and running before they start the desktop UI, but the UI process
itself still only reads and mutates durable run state.

The driver heartbeat file now persists executable identity (`started_at`,
`executable_path`, `executable_deleted`, optional `stop_reason`) alongside the
PID. If the running driver notices that its executable has become a deleted
binary after an install or upgrade, it records `stop_reason=stale_executable`
and exits with failure so the OS supervisor can restart a fresh binary instead
of continuing to coordinate runs from obsolete code.

Those same coordination repair branches now have two repo-local verification
views: a handwritten Lean state machine under `proof/` and an Aeneas-generated
Lean translation of an extraction-friendly Rust model under
`proof/aeneas-check`. The generated package checks the concrete regression
cases for missing claims, orphan claims, claim/metadata drift, stale deleted
drivers, and claim-time driver context.

The local hook still runs through `.githooks/pre-commit`, and remote
verification now reuses the same `scripts/verify-engineering-gate.sh`
entrypoint from `.github/workflows/engineering-gate.yml` with a diff range so
only contract-relevant changes dispatch their focused checks in CI.

The repo now also has a versioned pre-commit gate path:

- `contracts/engineering-gate.toml`
- `scripts/verify-engineering-gate.sh`
- `scripts/verify-coordination-gate.sh`
- `.githooks/pre-commit`

That gate is repo-aware, keys off the staged file set, and uses focused
release-mode workflow/coordination tests instead of a blanket `cargo test`
before every commit.

Freeform-council write claims now add a stricter broker policy on top of that
selection-driven runtime: any council claim that reaches 3 minutes of age is
revoked on the next driven turn, and the speaker who lost it cannot reclaim
that same scope again for the rest of the run.

## Execution Environments

- `host` runs provider tasks directly on the machine and detaches the worker
  with `setsid`.
- `runsc` runs stage only the explicit workspace include paths into a
  run-scoped shared sandbox workspace, reuse that sandbox across survey and
  provider turns, and materialize completed file writes back into the host
  checkout from artifact snapshots.

The selected environment is stored on the run and reused for later task
execution, built-in survey tasks, and council-provider turns.

## Storage Layout

The authoritative coordination state lives under the shared coordination spool:

```text
coordination/
  <run_id>/
    run.json
    events.jsonl
    interrupts/
    tasks/
    claims/
    artifacts/
```

- `run.json` stores run-wide configuration and status.
- `interrupts/` stores durable policy pauses and their eventual resolutions.
- `tasks/` stores the current task snapshots.
- `claims/` stores claim ownership and heartbeat metadata.
- `artifacts/` stores task outputs that later tasks consume.
- `artifacts/<task_id>/history/generation-<n>/` stores archived artifact files
  from superseded restart generations; the top-level artifact files remain the
  active generation only.
- `artifacts/<task_id>/contract.json` stores typed contract metadata when a
  provider task completed through the typed-envelope path.
- `events.jsonl` is the audit trail shown in the UI.
- `events.jsonl` now also records staged-rail decisions (`rail_decision`) so
  blocked sync-back and pre-launch prompt rewrites are visible in the same
  durable event stream as claims, retries, and interrupts.

The run-detail "Recent Events" panel is a rolling view over the newest 40
events from `events.jsonl`, not the oldest 40. Task counters come from the live
task snapshots, so the event list and task-status section are fed by different
reads over the same durable spool.

### SQLite Read-Side Index

A shared SQLite database (`~/.rust-chatbot/index.db`) acts as an optional
read-side index over the authoritative file storage.  The database is owned by
the `chatbot_lib` `StorageIndex` type and opened on demand via
`get_storage_index()`.

**Session tables** (created by `chatbot_lib::storage::index::ensure_schema`):
`session_index`, `session_fts` (FTS5), and `schema_meta`.  These are fully
populated and queried at runtime.

**Coordination tables** (`runs`, `tasks`, `coordination_event`,
`provider_usage`): dual-write helpers exist in
`src/coordination/parts/storage_core.rs` (`index_run`, `index_task`,
`index_coordination_event`, `index_provider_usage`).  These INSERT calls
execute against whichever schema the shared `StorageIndex` connection provides.
The coordination table DDL is defined in the canonical database design doc
(`docs/autonomous/database-doc.md`) but has not yet been added to the runtime
`ensure_schema` path — until it is, the dual-write calls fail silently and the
file spool remains the only queryable coordination store.
When those tables do exist, task dual-write now binds every declared column in
the `tasks` insert/update statement again, so the optional task index no longer
drops rows because of a parameter-count mismatch.

The index is fully rebuildable from the filesystem via
`rebuild_coordination_index()` (which delegates to `StorageIndex::rebuild_all`).
If `index.db` is deleted or unavailable, the application falls back to directory
scans for all read paths.  See `docs/autonomous/database-doc.md` for the full
schema, versioning policy, and rebuild expectations.

## Shared Runtime Root

Autonomous provider launches now resolve shared runtime config from
`~/.rust-chatbot/` first. That includes the shared `.mcp.json`, the main
`config.toml`, provider API keys, and provider-sidecar directories such as
`.claude-desktop/`, `.codex/`, `.gemini/`, `.minimax/`, `.nvidia/`,
`.openrouter/`, and `.qwen/`.

During the split migration, the shared runtime still falls back to legacy
repo-local config files when the canonical home file is missing. The canonical
migration script now lives in
`../chatbot-worker/scripts/migrate-home-state-to-shared-root.sh`.

## Workflow Modes

Each mode seeds a distinct initial task set and uses its own expansion hooks to
build up the task DAG dynamically as tasks complete.

### `SingleProvider`

Seeds a single provider-backed task. When follow-up generated tasks exist, the
coordination engine keeps them on the run's selected provider rather than
honoring planner-supplied provider overrides. The simplest path through the
coordination engine is still one provider call, one artifact.

### `DesignConsensus`

Seeds a `SurveyWorkspace` task followed by a `DraftDesignDoc` task. Once the
draft completes, a `ConvergeDesignDoc` task is spawned to iterate across
providers until consensus is reached, then a `MaterializeDesignDoc` task writes
the final document to disk.

The autonomous provider registry now includes the current seven autonomous
providers, including `nemotron`. Design-consensus seeding reads that central
registry, and run creation refreshes the workspace-local
`workflow-registry/providers.json` copy when it is stale so provider prompts
see the same current provider set. The worker-side provider dispatcher now
maps those same autonomous names into `chatbot_lib::ModelProvider`
values, so seeding and execution stay aligned. That same registry now also
stores `structured_output_mode`, which the worker consumes at execution time:
Claude-compatible providers receive inline `--json-schema` contracts, and
Codex-compatible providers receive persisted sidecar schema files via
`--output-schema`, including `runsc` mounts when the task is sandboxed.

### `DesignImplementation`

Seeds `SurveyWorkspace` and `PlanFlatTasks`. The planning task produces a flat
list of generated read and write tasks. `PlanFlatTasks` expansion writes those
tasks durably, assigns providers in registry-order round robin, and rewires the
`ReviewArtifacts` and `ApplyApprovedEdits` tail tasks to depend on them. When a
generated write task is clearly the bootstrap or scaffold step for the codebase,
later generated write tasks also depend on that bootstrap task so the workspace
foundation lands before downstream implementation slices run.

### `LightweightImplementation`

Seeds `SurveyWorkspace`, an implementation `PlanFlatTasks`, a validation
`PlanFlatTasks`, and a `ReviewArtifacts` tail task.

The workflow is configured from note-backed UI fields:

- `lightweight_task_splitter` selects the provider/model used for
  implementation task splitting.
- `lightweight_validation_model` selects the provider/model used for the
  validation planning and final validation review tasks.
- `lightweight_worker_models` is the round-robin worker pool for generated
  implementation and follow-up fix tasks. The UI now edits that pool through
  repeatable per-row provider dropdowns with inline add/remove controls.
- `lightweight_task_doc` stores the active task-list document path.
  The create-run form leaves both this path and the design-doc path as plain
  editable text fields without forced placeholder content.

The implementation planner can start from the run goal alone. If the canonical
design doc or active task list does not exist yet, that first planning pass
creates both documents at the configured paths and then generates the initial
implementation fragments. Once those fragments are expanded, the downstream
validation planner is rewired to depend on them, so it only runs after the
current implementation pass completes. That validation planner and the final
review use `lightweight_validation_model`, which defaults to the task splitter
when the note is absent on older runs.

The validation planner then generates read-only validation tasks. Once those
validation tasks complete, the final validation review checks the current
workspace against the design doc and task list. If more work remains, it writes
the next task-list document, updates the run's active task-list path in
`run.notes`, and emits smaller follow-up fix tasks for the worker pool. If no
follow-up tasks are emitted, the run completes on the final validated task
list.

Unlike `DesignImplementation`, this workflow does not route worker claims
through provider-capability fallback and does not rely on per-task artifact
acceptance or file-tracking to advance the loop. The configured splitter,
worker, and validation providers are treated as the source of truth, and the
review pass is the only validation gate that decides whether another pass is
needed. Lightweight planning and review tasks still use typed JSON envelopes,
but generated read/write worker tasks report completion through plain-text
status summaries plus snapshot-derived workspace state instead of the standard
task-report JSON contract. Claim scheduling still blocks overlapping
source-file writes, but lightweight generated write tasks may overlap on the
shared active task-list document so multiple workers can progress through the
same pass in parallel. The planner can also declare explicit intra-pass task
ordering with `depends_on_titles`, which the runtime resolves into durable
task dependencies before any worker claims begin. Lightweight review passes
also skip the normal review-approval interrupt gate, so follow-up fix tasks
queue immediately instead of pausing for operator input.

### `GithubBranchFix`

Uses a dedicated single-worker branch-fix loop rather than the full
`LightweightImplementation` configuration surface. The create-run form exposes
one worker-model selector through `run.provider`, hides task-list/splitter/
validator settings, and forces `max_agents = 1`.

The planner bootstraps a design doc when needed, adds branch-specific GitHub
Actions coverage, ensures a versioned pre-commit hook runs tests, then emits
worker tasks that poll the latest GitHub Actions run for the target branch.
When a run fails, that worker inspects the failing run directly, fixes the
branch, adds or updates tests, commits, pushes, and does not report completion
until it confirms a newer GitHub Actions run has completed successfully for
that branch after the push. The branch-fix prompt contract now also requires
`gh auth status` before any `gh run list` or other GitHub CLI API call so
polling does not silently fall back to GitHub's unauthenticated rate limit
bucket; when supported it prefers `gh auth status --active`, but older `gh`
builds fall back to plain `gh auth status`. The input rail rejects branch-fix
poll/fix/review tasks before provider execution when that auth check fails.

Validation still uses typed plan/review tasks plus plain-text generated worker
tasks, but follow-up passes emit fix tasks directly from the current workspace
state instead of advancing a task-list document. Completion is only allowed
once the review can cite workflow coverage, local test evidence, and the
observed post-push GitHub run success. The workflow-specific note-backed
settings are `github_branch_fix_branch` and `github_branch_fix_poll_minutes`.

### `RalphLoop`

Seeds a `RalphPlan` task. Each plan completion spawns a `RalphExecute` task,
and each execute completion spawns a `RalphVerify` task. Verify spawns the next
`RalphPlan` if the goal is not yet satisfied, forming an iterative loop. The
loop terminates when Ralph declares success or the iteration limit is reached.

### `FreeformCouncil`

Seeds an initial council turn. Expansion hooks drive a ledger-backed council
where multiple providers take turns contributing, critiquing, and refining a
shared output. Turn policy still provides the fallback routing behavior, but
explicit `[HANDOFF ...]` actions now take precedence and `[REQUEST_USER ...]`
actions pause the run through the same durable interrupt model used elsewhere.
Council write scopes are broker-granted, auto-revoked after 3 minutes if they
go stale, and permanently blocked from reacquisition by the same speaker once
that stale revocation happens. The council state on disk now also carries a
persisted `termination_policy`, seeded from the run's turn cap, so max-turn,
max-stall, and all-models-voted-complete rules are inspectable and drive the
runtime directly.

### `CodeReview`

Seeds `SurveyWorkspace`. The survey includes automated tool checks (linters,
formatters, test runners). A `PlanCodeReview` built-in task then decomposes the
review scope into per-chunk review tasks. Those review chunks are assigned in
registry-order round robin rather than being pinned to the run's top-level
provider. The planner also reads `docs/review-profile.md` as repo-specific
architecture context and emits a warning when that profile is missing or stale.
That review profile is the canonical repo-specific review contract; dated files
like `docs/code-review-2026-04-11.md` are archived review outputs and should
not be treated as the current architecture source of truth.
A `MaterializeDesignDoc` tail task aggregates findings into the run's
configured review-document path under `docs/`, which uses the workflow-derived
hashed default unless the operator overrides it.

### `PerfHillClimber`

Seeds two initial tasks that run in parallel:

- **`PerfBaseline`** — provider task that runs `bench.sh` against the current
  workspace and writes `/tmp/perf_baseline.json` as a standardized benchmark
  artifact containing correctness and timing fields.
- **`PerfParse`** — provider task that reads the design doc(s) and outputs a
  JSON array of discrete, independently testable performance improvement ideas.

Once `PerfParse` completes, `expand_perf_parse_task` spawns the first idea
triplet gated on both baseline and parse finishing. Each idea runs through:

1. **`PerfApply[N]`** — provider applies the code change and runs `cargo check`.
2. **`PerfBench[N]`** — does a full `cargo build --release` then runs `bench.sh`,
   writing `/tmp/perf_bench_N.json` in that same standardized artifact format.
3. **`PerfEvaluate[N]`** — compares bench results against the baseline. If the
   bench artifact reports `correctness.status = fail`, the change is rejected
   immediately. Otherwise, if the improvement is ≥ 2% on the artifact's
   primary-case median (falling back to legacy `parallel_median_s` when needed),
   the change is kept; otherwise `git checkout -- . && git clean -fd` reverts
   tracked changes and removes untracked files before the next idea.

`expand_perf_evaluate_task` then spawns the next idea's triplet (gated on the
current evaluate completing), forming a sequential hill-climbing chain. Each
apply step operates on whatever workspace state the prior evaluate left behind,
so kept improvements are cumulative. The baseline is measured once and reused
for all comparisons so the threshold is always relative to the original state.
If an idea-scoped perf task fails (`PerfApply`, `PerfBench`, or
`PerfEvaluate`), the coordinator treats that idea as rejected instead of
failing the run. It cleans the perf worktree, synthesizes any missing bench or
evaluate artifacts, marks the idea reverted, and advances to the next idea.
Only baseline/parse failures still terminate the perf run.

The scan path for `bench.sh` defaults to `/snap` and can be overridden by
adding `bench_scan_path=<path>` to the run's notes field. The create-run UI now
also exposes a dedicated bench-script input for `PerfHillClimber`; it persists
as `bench_script_path=<absolute path>` in the run notes and overrides the
workspace-local `./bench.sh`. Goal-text parsing of an absolute `.../bench.sh`
path remains as a fallback for older runs.

The standardized perf benchmark artifact is versioned (`schema_version = 1`)
and includes:

- `correctness.status`: `pass`, `fail`, or `unknown`
- `correctness.checks[]`: optional named correctness checks
- `metrics.primary_case`: the benchmark case used for keep/revert decisions
- `metrics.cases[]`: per-case `median_s` and `min_s` timings
- legacy flat timing fields copied forward during migration so older
  benchmarks remain readable while scripts move to the new schema

Perf runs now provision a detached disposable git worktree under the shared
coordination area and execute apply/bench/evaluate steps there instead of in
the source checkout. The run still stores the source repository as
`workspace_path`, while perf task `cwd` values and prompts target the
run-scoped worktree path recorded in the run notes. The disposable worktree is
created from `HEAD`, so uncommitted source changes are intentionally excluded.
Before perf tasks run, local Cargo path dependencies inside the worktree are
rewritten to absolute source paths so sibling-repo dependencies continue to
resolve correctly. The disposable worktree is removed when the run stops or
reaches a terminal state.

## Deployment Scripts

All operational scripts live under `scripts/`:

- **`launch.sh`** — Finds and runs the installed `auto-runs` binary from `$PATH`
  or cargo bin. Before launching the UI, it ensures the background driver
  service is running.

- **`start-auto-runs.sh`** — Reinstalls `auto-runs` and `auto-runs-worker`,
  ensures the background driver service is running, then launches the UI in the
  background and writes the UI log under `~/.rust-chatbot/logs/`.

- **`restart-auto-runs.sh`** — Resolves the repo root from the script location,
  reinstalls both binaries, ensures the background driver service is running,
  stops only existing UI instances, then launches a fresh UI instance in the
  background. The process match excludes `drive-forever`, so UI restart no
  longer kills the driver service.

- **`scripts/driver-service.sh`** — Explicit OS-service management wrapper plus
  shared shell helpers. `just` invokes it with `install-binaries`, `install`,
  `uninstall`, `start`, `stop`, `restart`, `status`, or `logs`.
  `install-binaries` only refreshes the installed `auto-runs` and
  `auto-runs-worker` binaries, while `install` also installs or refreshes the
  OS-native driver service. The script writes the systemd user unit or launchd
  plist directly and does not invoke the UI binary in app mode. On Linux, the
  generated systemd unit now uses `KillMode=process` plus `SendSIGKILL=no` so
  restarting the driver service does not kill already-adopted detached worker
  processes that still own active task claims. The same helper also refreshes an
  installed service definition when its generated unit/plist content drifts, so
  shell-only driver-service fixes apply on the next `just start` or
  `just restart`.

There is only ever one `auto-runs` instance running at a time. The repo-root
`just install`, `just start`, and `just restart` recipes are the preferred
entrypoints for iterative development; they delegate to these `scripts/`
helpers directly now that the old root `start.sh` and `restart.sh` wrappers
are gone.

## Restart Behavior

There are two distinct restart paths:

- `restart_failed_tasks(...)` — the "Restart Failed" button. Requeues failed or
  completed tasks that need retry, respecting `auto_retry_failed_tasks` and
  `max_attempts`. The run status may become `Active` if tasks are requeued.
- The desktop "Restart Failed" popup can optionally pick one task, reassign its
  `assigned_provider`, and, for a completed task, mark it failed immediately
  before the run-level restart is applied.

- `mark_task_failed_for_restart(...)` — the "Mark Failed" action. Manually marks
  a specific task as failed for later manual restart. Sets
  `auto_retry_failed_tasks = false` on the run to prevent auto-retry, then
  reloads the snapshot. The run status becomes `Failed`.

## Build Configuration

The checked-in `.cargo/config.toml` selects `clang` with `mold` for Linux
x86_64 builds and limits Cargo to one build job. It does not set a shared
`target-dir`; each checkout uses its own default Cargo output directory unless
the caller explicitly overrides it.

## Related Docs

- [Autonomous Coordination](autonomous-coordination.md)
- [GitHub Branch Fix](github-branch-fix.md)
- [Lightweight Implementation](lightweight-implementation.md)
- [Autonomous Design](../autonomous/design.md)
- [Coordination Notes](../autonomous/coordination.md)
- [Database Design](../autonomous/database-doc.md)
- [Boundary](../boundary.md)
