# Autonomous Coordination

This document describes the current execution architecture for autonomous runs
inside `auto-runs`.

## Run Creation

The desktop UI creates runs through `coordination::upsert_run(...)`. Each run
stores:

- workflow mode
- workspace path
- provider
- execution environment (`host` or `runsc`)
- optional `runsc` workspace include paths
- review and council policy fields

That state is durable from the start, so retries and later UI refreshes do not
depend on transient in-memory state.

The right-hand Runs sidebar now starts each run row with a color-coded status
token, then prepends compact durable run metadata from `run.json` before the
goal text: workflow value in a darker light green, workspace path in a darker
light blue, and execution environment in a lighter grey than the surrounding
text. Each run row is clamped to a single visual line with truncation
ellipsis, but it expands to reveal more of the same line automatically as the
sidebar gets wider. Workspace values are shortened in the
sidebar by stripping that shared prefix. The status token uses terminal-state
coloring: dark green for completed runs and a brighter dark red for failed
runs.

If a task is about to execute and its configured workspace directory does not
exist yet, the coordinator creates that directory on demand and records a
`task_workspace_created` event in the run ledger before continuing.

If the user leaves the design-doc path at its generated default, run creation
materializes it as `docs/autonomous/<workflow>-doc-<hash>.md`, where
`<workflow>` comes from the workflow mode and `<hash>` is a short stable hash
of the run ID.

`CodeReview` uses that same run-level path for its final review document rather
than switching to a separate date-based filename during materialization.

`PerfHillClimber` is currently disabled at run-creation time. The workflow code
still exists on disk, but the UI and validation layer reject creating new perf
runs until that path is fully wired back into the supported coordination DAG.

## Task Lifecycle

The coordination layer seeds and expands tasks into the durable spool. A task
normally moves through:

- `queued`
- `claiming`
- `running`
- `artifact_ready`
- `completed` or `failed`

Claiming is lease-based. The claimer must still satisfy dependency checks and
must not overlap another live task's `write_scope`.

Provider-backed execution now also has a hard per-task runtime budget. Seeded
tasks receive a default estimate by task kind, and generated plan/review tasks
must carry an explicit `estimated_minutes` value. The worker enforces that
budget through the provider stop signal; if the provider subprocess runs past
the estimate, the attempt is terminated and the normal requeue-or-fail retry
path decides whether another worker should retry the same task.

If a task snapshot is still `running` or `claiming` but its worker-state file
has disappeared, claim-state refresh treats that as durable corruption only
after the recorded owner PID is no longer alive or the five-minute heartbeat
window has elapsed. Otherwise the repair is deferred so a live worker can
finish rewriting or recreating its own state. Once the task is genuinely stale,
the coordinator emits `task_claim_missing`, clears the stale owner metadata,
and then requeues or fails the task based on the normal attempt budget. That
repair pass takes a run-scoped lock and reloads the current task snapshot
before emitting missing-claim recovery events, so a second reader cannot append
duplicate `task_requeued` repairs from stale state after another process
already moved the task to `queued`, `failed`, or `completed`.

Claim refresh also repairs the inverse drift case. If a worker-state file is
still on disk but the durable task snapshot is no longer `running` or
`claiming`, the coordinator reloads the live task snapshot before treating the
file as orphaned. Only then does it remove the orphaned worker-state file and
record `task_claim_orphaned`, which prevents a stale pre-claim snapshot from
deleting a newly created worker record.

For active task/claim pairs, reconciliation now treats heartbeat movement as
ephemeral. The background driver polls every 500ms, while provider workers
heartbeat their adopted claims roughly every 60 seconds. Because the claim JSON
and task JSON are written separately, a reader can briefly observe a newer
heartbeat timestamp before the matching task snapshot lands. Claim refresh only
repairs durable drift in that situation when ownership changes or the task-side
stale deadline is missing or already expired. Heartbeat-only skew on a still-
live active task is left alone so the UI and driver do not spam
`task_claim_reconciled` events during normal heartbeats.

Heartbeat staleness is still enforced when a run is driven, not by a separate
global sweeper. The difference now is that driving is no longer tied to an open
UI window: a separately installed background driver service polls the
coordination spool, heartbeats its own status file, and drives every
nonterminal run on each interval. The desktop UI only reads persisted run
state, so stale workers and newly runnable tasks advance without requiring the
operator to keep the window open, but only while that service is actually
installed and running.

Maintenance passes that unblock dependency-ready tasks now persist only the
task files they actually changed. They do not rewrite the full in-memory task
slice back to disk, which prevents a stale snapshot from stamping an active
worker-owned task back to an older `queued`, `running`, or `completed`
representation while another process is heartbeating or finalizing it.

Detached provider workers now also have a launcher-side watchdog that enforces
both liveness and runtime budgets. After a worker adopts a claim and starts
task execution, the launcher polls the child process and its claim file
together. If the worker is still alive but has gone five minutes without a
fresh heartbeat, the launcher terminates that worker immediately and records
the normal worker-exit requeue or failure path instead of waiting for the
background driver to notice later. The same launcher watchdog also kills a
detached worker that exceeds the normal provider task timeout derived from the
task's `estimated_minutes`.

The background driver now also self-checks its executable identity on every
poll. If the running process has become a deleted or stale binary after an
install or upgrade, it writes that state into `background-driver.json` and
exits with failure so a fresh `auto-runs drive-forever` process can take over instead of
continuing to coordinate runs from an obsolete executable.

Driver status now persists enough identity to debug ownership mismatches
directly from disk:

- `pid`
- `started_at`
- `executable_path`
- `executable_deleted`
- optional `stop_reason`

When a task is claimed by that background driver, the same identity summary is
also appended inline to the `task_claimed` event message. That keeps run-local
forensics self-contained: a stale deleted driver can be diagnosed directly from
the affected run ledger even if the shared `background-driver.json` has already
been overwritten by the replacement driver.

The repo now also keeps an extraction-friendly Rust model of those repair
branches under `proof/aeneas-model`. A companion generated Lean package under
`proof/aeneas-check` is rebuilt from that Rust model and checks the same
regression transitions that motivated the fixes: missing-claim requeue or
fail, orphan-claim cleanup, claim-owner reconciliation, stale-driver shutdown,
and claim-time driver identity breadcrumbs.

For normal operations, that driver should be installed as an OS-native user
service that runs the same `auto-runs drive-forever` entrypoint:

- Linux installs `~/.config/systemd/user/auto-runs-driver.service`
- macOS installs `~/Library/LaunchAgents/com.mjgil.auto-runs-driver.plist`

Those service definitions are written and managed by repo shell scripts rather
than by the desktop UI. The normal app-launch scripts ensure the service is
installed and running before they launch the UI process.

Freeform-council claims now have a broker-side hard timeout of 3 minutes from
`claimed_at`. When a driven council turn sees a claim at or past that TTL, it
revokes the claim immediately, records a `claim_revoked` ledger/event entry,
and blocks that same speaker from reclaiming the same scope again during the
run. Other speakers may still claim the revoked scope and continue the work.

## Typed Contracts And Reasks

Provider-backed contract tasks now use a typed task-artifact envelope instead
of relying only on task-kind-specific prompt discipline:

- `PlanFlatTasks` and `PlanCodeReview` use `payload_kind = plan_tasks`
- `ReviewArtifacts` uses `payload_kind = review_findings`
- `DraftDesignDoc` and `ConvergeDesignDoc` use `payload_kind = design_doc`
- `PerfParse` uses `payload_kind = perf_ideas`

`GithubBranchFix` is the one current exception for `PlanFlatTasks`: its
implementation and validation planners intentionally stay on the lightweight
plain-array task-list format, so the runtime skips typed-envelope schema
selection and contract reasks for those planner tasks only.

The top-level envelope carries the normal task outcome fields plus
`payload_kind`, `contract_version`, and `payload`. Contract validation runs
inside the same claimed execution attempt. If the provider returns malformed
structured output, the worker appends a `task_contract_reask` event and retries
in-process instead of burning a full task attempt on a schema-only mistake.

Most provider task reports still carry `changed_files` and the normal
acceptance checks continue to validate them. `LightweightImplementation` now
uses a separate runtime policy: it keeps the configured providers instead of
category-based fallback routing, skips the standard artifact-acceptance rails
for lightweight task completions, and does not persist lightweight
`changed_files` or materialized-file tracking as durable coordination state.
The lightweight loop advances on the final review pass deciding whether to emit
another batch of tasks. Lightweight planning and review tasks keep their typed
JSON envelopes, but generated lightweight read/write worker tasks use plain
text completion summaries plus snapshot-derived workspace state instead of the
standard task-report JSON contract. Lightweight generated write tasks may also
share overlapping `write_scope` values without blocking each other at claim
time. In lightweight mode, patch-level git publish is the merge boundary for
generated write tasks, so claim-time scope locking is no longer used to
serialize those writes. Write-scope locking still applies to non-lightweight
write tasks.

Successful typed artifacts persist contract metadata at
`artifacts/<task_id>/contract.json`. Downstream consumers still read the
existing task-specific artifact bodies (`generated_tasks` arrays, review
artifacts, markdown documents, perf-idea objects) so older expansion and
materialization paths remain compatible while the provider boundary hardens.
Generated plan and review task slices now also accept `depends_on_titles`,
which lets providers describe intra-plan dependency edges using sibling task
titles before the coordinator resolves those edges into task IDs. They also
carry `estimated_minutes`, which is persisted onto the durable task and later
used as the worker runtime timeout budget.

Raw generated `read_scope` and `write_scope` entries are validated before any
internal scope normalization runs. That trust-boundary check rejects absolute
paths and `..` traversal in the original planner/review payload instead of
letting the relative-scope canonicalizer strip a leading slash and accidentally
reinterpret an absolute path as workspace-relative.

Contract-backed provider execution now also uses the provider capability
registry to select a real structured-output mode instead of treating the
contract as prompt text only. `workflow-registry/providers.json` carries
`structured_output_mode` per provider. When that mode is
`provider_native_json_schema`, the worker builds a JSON Schema directly from
the typed envelope contract and passes it through the shared provider bridge.

Today that native path is wired for both provider families that expose schema
flags through the existing CLI bridge:

- Claude-compatible launches pass the envelope schema inline through
  `--json-schema`.
- Codex-compatible launches persist the schema as a shared-infra sidecar file,
  mount it into `runsc` when needed, and pass the mounted path through
  `--output-schema`.

Providers without native schema support still use prompt-only contract
instructions. Regex-constrained mode is defined in the capability model, but
for contracts that embed markdown inside JSON it currently downgrades to the
same prompt-only fallback instead of pretending regex enforcement exists.

## Interrupts And Policy Waits

Policy pauses are now durable spool objects instead of implicit state:

- run-scoped interrupts live under `interrupts/<interrupt_id>.json`
- any open interrupt makes `load_run_snapshot(...)` surface
  `WaitingForPolicy`, including freeform-council runs
- `drive_run_for_instance(...)` and `claim_next_runnable_task(...)` both stop
  before executing new work while any interrupt remains open

Each interrupt writes `interrupt_opened` and `interrupt_resolved` events to the
run ledger so the UI and postmortem event stream show why the run stopped and
when it resumed.

Two resolution side effects are wired today:

- rejecting or deferring a `ReviewApproval` interrupt fails dependent
  `ApplyApprovedEdits` tasks immediately instead of leaving them implicitly
  blocked behind review policy
- resolving a `CouncilUserReply` interrupt injects a durable
  `UserInterjection` entry into the council ledger, which reactivates the
  council run from the same persisted interrupt object

## Council Handoffs And Termination

Freeform-council turns now use explicit control actions instead of relying only
on mention heuristics inside plain discussion text.

The prompt teaches two brokered control markers:

- `[HANDOFF @speaker reason]` requests an explicit delegation to another model
- `[REQUEST_USER question]` records a blocking user-decision request

Those actions persist as first-class ledger entries:

- `handoff_requested`
- `handoff_accepted`
- `user_input_requested`

Speaker selection now follows a stable order:

1. pending explicit handoff target
2. unresolved user request blocks the council
3. existing turn policy (`round_robin`, `mention_driven`, `adaptive`)

Plain `@mentions` still influence the fallback selectors, but they no longer
override an explicit pending handoff.

`CouncilRun` also now persists a `termination_policy` object on disk. Seeded
freeform-council runs derive that policy from the configured turn cap, so the
visible policy and the runtime limit stay aligned. The runtime evaluates that
policy both before selecting the next speaker and after each completed turn:

- `MaxTurns(N)` fails the council when turn count reaches `N`
- `MaxStallTurns(N)` fails the council when no real progress persists for `N`
  turns
- `AllModelsVotedComplete` moves the council into broker verification instead
  of completing purely on transcript markers

If verification fails after all votes are in, the broker records
`votes_rejected`, clears the completion votes, and returns the council to
`Active` so the next turn can continue from durable state.

## Staged Rails

Provider-backed tasks now run through four lightweight rails instead of
trusting prompt discipline alone.

- `Input` runs before workspace prep and provider launch. It rejects empty
  task prompts, malformed write-task config, write tasks when the run disables
  writes, and missing canonical design docs for task kinds that require the
  current design artifact to exist already.
- `Dialog` runs after the final provider query is assembled. It can rewrite the
  query to append missing contract scaffolding and an explicit write-scope
  reminder before the provider CLI sees the prompt.
- `Execution` runs in `complete_task(...)` after materialized files are
  captured but before sync-back. It rejects sync-back when files fall outside
  the declared write scope, when a mutating task did not declare the `edit`
  tool, or when `ApplyApprovedEdits` tries to write before review approval has
  been durably resolved.
- `Output` runs alongside artifact validation and rejects changed files that
  still contain blocked stub markers, using the same code-review
  materialization exception as artifact validation.

Every non-allow decision appends a `rail_decision` event to the run ledger, so
Recent Events and postmortems show whether a task was rewritten, rejected, or
interrupted at a specific stage. If a rail returns `Interrupt`, the runtime
also creates a durable interrupt object before bailing so the UI sees the same
`WaitingForPolicy` behavior used by review approval and council user-input
pauses.

## Launch Paths

There are two main execution paths:

- **Built-in handlers** run inside the `auto-runs` process for coordination
  operations: workspace survey, code-review planning, review materialization,
  design-doc materialization, and perf-hill-climber baseline/bench tasks.
- **Provider-backed tasks** launch `auto-runs-worker`, which adopts the
  launcher claim, performs the actual CLI provider call, and writes results
  back into the run spool.

Worker resolution checks the current executable's companion directory first.
If `auto-runs` is running from an installed path such as `~/.cargo/bin`, it
falls back to the repo-local `target/<profile>/auto-runs-worker` location
derived from the `auto-runs` checkout. Mixed debug/release launches are still
rejected.

This keeps the durable coordination model separate from provider execution
details while still reusing the shared provider bridge from the sibling
`chatbot-worker` repo (`chatbot_lib`).

The worker-side autonomous provider dispatcher now accepts the full current
registry set: `claude`, `glm`, `minimax`, `qwen`, `codex`, `nemotron`, and
`gemini`. Registry additions must be reflected in that
dispatcher as well as in `workflow-registry/providers.json`.

The provider launch handoff is now:

1. `auto-runs` claims the task under the orchestrator PID.
2. `auto-runs-worker autonomous <run_id> <task_id> <provider> <launcher_pid>`
   starts as a detached child for already-claimed provider tasks. The separate
   FIFO pull path is reserved for future pooled workers; detached per-task
   launches must not go back through queue assignment after the driver already
   owns a concrete claim.
3. The worker immediately calls `adopt_claim_for_worker(...)`, rebinding the
   claim from launcher PID to worker PID before provider execution begins.
4. The worker heartbeats that adopted claim while the provider CLI runs.
   While the worker is still starting, the launcher now bridges claim
   heartbeats on the launcher-owned claim until adoption succeeds or the child
   exits, so the pre-adoption window is heartbeat-driven instead of depending
   on a lease field.
5. If the worker fails before claim adoption succeeds, it calls
   `release_claim_after_launch_failure(...)` so the run records a launch
   failure event instead of waiting for a lease expiry.
6. If the worker fails after claim adoption but before normal provider-task
   finalization clears the claim, it now calls
   `release_claim_after_worker_failure(...)` against the worker-owned claim so
   the task is requeued or failed immediately instead of expiring under the
   stale worker PID.

Claim snapshots and events now also preserve the worker handoff lifecycle so
lease-expiry forensics no longer stop at a bare PID:

- `TaskClaim` stores `lifecycle_phase`, `worker_pid`, `worker_spawned_at`,
  `worker_adopted_at`, `execution_started_at`, `worker_exited_at`, and
  `worker_exit_detail`
- the run ledger records `task_worker_spawned`, `task_worker_started`,
  `task_worker_adopted`, `task_execution_started`,
  `task_worker_adopt_failed`, and `task_worker_exited`
- lease-expiry requeue/fail messages now embed the persisted claim lifecycle
  summary so operators can see whether the launcher never handed off, the
  worker adopted but never started execution, or the worker exited before the
  task reached a terminal artifact

`auto-runs-worker` now returns `ExitCode` instead of forcing
`std::process::exit(...)` on failure paths. That preserves rust-debug log
flushes on startup/adoption failures, which makes the per-worker logs under
`~/.local/state/auto-runs-worker/` a reliable secondary source behind the run
spool.

Provider-backed task execution now also enforces a Qwen-specific hard timeout
of 20 minutes inside the worker. When that timeout fires, the worker stops the
provider subprocess and records a direct provider-task failure instead of
leaving the attempt running until an external stop. Worker-side provider
failure classification also treats `qwen-code command failed` output and raw
CLI spawn failures as direct provider-command failures rather than normalizing
them into malformed task-report artifacts.

For `runsc` launches, the detached worker still starts on the host and adopts
the claim before any sandbox handoff occurs. The launcher only passes the
autonomous `runsc` env hints (`RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV` and
`RUST_CHATBOT_AUTONOMOUS_RUNSC_WORKSPACE_INCLUDES`) into the worker.

The actual `runsc` transition now happens later, inside provider execution
after claim adoption. That keeps lease ownership, heartbeats, and launch-failure
cleanup in a normal host process while still making the provider CLI itself use
the shared autonomous sandbox workspace.

## Task Kinds

The full set of task kinds, grouped by workflow:

**Shared / built-in**
- `SurveyWorkspace` — surveys the workspace directory tree (built-in)

**DesignImplementation / DesignConsensus**
- `DraftDesignDoc` — initial design draft (provider)
- `ConvergeDesignDoc` — convergence pass across providers (provider)
- `MaterializeDesignDoc` — writes final doc to disk (built-in or provider)
- `PlanFlatTasks` — decomposes design into read/write tasks (provider)
- `GeneratedReadTask` — provider-backed read pass (provider)
- `GeneratedWriteTask` — provider-backed write pass (provider)
- `ReviewArtifacts` — aggregates read/write artifacts for review (provider)
- `ApplyApprovedEdits` — applies reviewed edits to disk (provider)

**CodeReview**
- `PlanCodeReview` — decomposes review scope into chunks (built-in)
  *(MaterializeDesignDoc is reused for the final review document)*

**RalphLoop**
- `RalphPlan` — plan the next iteration (provider)
- `RalphExecute` — execute the plan (provider)
- `RalphVerify` — verify the result and decide to loop or terminate (provider)

Ralph execution and verification templates are now written to run-scoped temp
paths (`/tmp/ralph_execute_template_<run_id>.json` and
`/tmp/ralph_verify_template_<run_id>.json`). Prompts, read scopes, write
scopes, and fallback parsing all use those same run-specific paths so
concurrent Ralph runs do not collide on shared template files.

**PerfHillClimber**
- `PerfBaseline` — measure the baseline benchmark (provider)
- `PerfParse` — parse design docs into a list of improvement ideas (provider)
- `PerfApply` — apply one idea and cargo-check (provider)
- `PerfBench` — release-build and benchmark the applied change, writing a
  standardized perf artifact with correctness plus timing fields (provider)
- `PerfEvaluate` — compare against baseline, keep or revert with
  `git checkout -- . && git clean -fd` on rejection (provider)

Idea-scoped perf failures (`PerfApply`, `PerfBench`, `PerfEvaluate`) are
recovered as rejected ideas rather than terminal run failures. The coordinator
cleans the perf worktree, synthesizes the missing bench/evaluate artifacts when
needed, records the idea as reverted, and advances to the next idea. Only
foundational perf failures such as `PerfBaseline` or `PerfParse` still fail the
whole run.

Manual restarts now recreate the disposable perf worktree before requeuing
failed or canceled perf tasks. That keeps restarted host-mode perf tasks from
launching against a deleted `cwd` after a canceled run cleans up its worktree.

`PerfParse` still requires the provider contract to return a JSON object with an
`ideas` array, but the parser now also salvages common numbered markdown
brainstorms when providers accidentally return prose instead of strict JSON.
That keeps format-only slips from failing the run before idea expansion starts.

For benchmark-producing tasks, `bench.sh` is now the authoritative artifact
producer. `PerfBaseline` and successful `PerfBench` runs tell the provider to
pass through the exact JSON emitted by `bench.sh` instead of reformatting it,
and invalid benchmark output is recorded as a malformed task report failure
instead of a soft "captured" success.

## Expansion Hooks

When a provider-backed task completes, `on_task_completed` in `claims.rs` fires
expansion hooks based on the task kind. Expansion hooks are how the DAG grows
dynamically rather than being fully pre-seeded:

| Completing task kind | Expansion action |
|---|---|
| `PlanFlatTasks` | Writes generated read/write tasks; rewires review/apply tail |
| `ConvergeDesignDoc` | Spawns `MaterializeDesignDoc` |
| `RalphVerify` | Spawns next `RalphPlan` (if looping) or terminates |
| `PerfParse` | Parses idea list; spawns first `PerfApply→PerfBench→PerfEvaluate` triplet |
| `PerfEvaluate` | Logs decision; spawns next idea triplet or marks run complete |

All other task kinds have no expansion hook — they are terminal within their
sub-graph and the run completes when all tasks reach a terminal state.
Completed tasks also clear `claimed_by` before the claim file is removed, so
terminal snapshots no longer look worker-owned after successful completion.

## Generated Provider Assignment

Generated task specs may omit `provider`; for non-`RalphLoop` workflows that is
the intended contract.

- `RalphLoop` is the only workflow that preserves an explicit generated-task
  provider override.
- `SingleProvider` and `FreeformCouncil` keep generated tasks on the run's
  selected provider.
- `DesignConsensus`, `DesignImplementation`, and `CodeReview` assign generated
  tasks in registry order with round-robin distribution.
- In `DesignImplementation`, if the generated plan includes a bootstrap or
  scaffold write task, later generated write tasks gain an explicit dependency
  on that bootstrap task instead of racing it in parallel.
- `CodeReview` review chunks follow the same round-robin assignment instead of
  pinning every chunk to `run.provider`.
- The canonical autonomous registry includes `nemotron` alongside the original
  provider set, and `DesignConsensus` refreshes the workspace-local registry
  copy when it has gone stale so prompt context stays aligned with the current
  central registry.

## `runsc` Execution

`runsc` is configured per run, not per individual task config.

When a run uses `runsc`:

- the UI persists `execution_environment = runsc`
- the run can also persist `runsc_workspace_includes`
- task launch exports `RUST_CHATBOT_AUTONOMOUS_EXECUTION_ENV=runsc`
- task launch exports `RUST_CHATBOT_AUTONOMOUS_RUNSC_WORKSPACE_INCLUDES` as a
  JSON array
- the coordinator initializes a run-scoped sandbox root at
  `<shared_infra_dir>/coordination/<run_id>/runsc-shared/workspace`
- provider-backed worker execution activates the autonomous runsc env inside
  the worker process, switches the worker `cwd` into that shared sandbox
  workspace, and builds the provider session from the sandbox-local task view
- built-in workspace survey, council turns, and provider-backed tasks all reuse
  that same run-scoped sandbox workspace
- `runsc_workspace_includes` is authoritative; only the explicit include
  entries are staged into the sandbox workspace
- direct sandbox `sync_back()` is disabled for autonomous runs so the container
  wrapper does not copy arbitrary workspace changes back into the host checkout
- Rust snapshots materialized files from the shared sandbox workspace after
  execution, constrained by the task's declared `write_scope`, and then writes
  those captured contents back into the host checkout after successful task
  completion

This means every model in a `runsc` run, plus the survey builtin, sees the same
shared sandbox filesystem state for that run. Files written inside the sandbox
remain visible to later turns in the same run even before the host checkout has
been updated.

For host execution, provider-backed tasks still execute against each task's
`cwd`, and Freeform Council turns now synthesize a provider session whose
`working_dir` is the run `workspace_path`. That keeps direct council provider
edits and file reads anchored to the selected run workspace instead of the
desktop app's own process cwd.

Host provider launches now also resolve Node-backed CLI wrappers (`claude`,
`gemini`, `qwen`) to concrete binaries before spawn and prepend that binary's
directory onto the child `PATH`. That keeps autonomous host runs working even
when the long-lived driver daemon was started from a stripped environment,
because `/usr/bin/env node` inside those wrappers still resolves at execution
time.

For `PerfHillClimber`, the benchmark command still defaults to `./bench.sh`
under the run workspace. The UI now exposes a dedicated bench-script input for
that workflow; `auto-runs` persists it as a `bench_script_path=...` setting in
the run notes, and the perf prompts use that script path when present instead
of assuming the workspace-local script exists. Goal-text parsing of an absolute
`.../bench.sh` path remains as a compatibility fallback.

Perf baseline and bench tasks now standardize benchmark output into a versioned
JSON artifact before writing `/tmp/perf_baseline.json` or
`/tmp/perf_bench_<idea>.json`. The contract carries both correctness and speed:

- `schema_version` and `benchmark` identify the artifact contract
- `correctness.status` is `pass`, `fail`, or `unknown`
- `correctness.checks[]` can record expected-vs-actual output summaries
- `metrics.primary_case` and `metrics.cases[]` carry benchmark timings
- legacy flat timing fields such as `parallel_median_s` are still copied for
  compatibility during the migration

`PerfEvaluate` now treats `correctness.status = fail` as an automatic reject.
If correctness is `unknown`, it falls back to the older timing-only behavior so
existing benchmark scripts can be migrated incrementally.

Perf runs also create a detached disposable git worktree from the source
workspace `HEAD` and route perf task `cwd` values plus apply/bench/evaluate
prompts to that worktree path. This keeps perf revert logic off the source
checkout while still letting the run reuse source-repo configuration such as an
absolute bench-script path. Before perf builds run, local Cargo path
dependencies in the worktree are rewritten to absolute source paths so sibling
repositories still resolve correctly. Perf provider tasks now also force the
provider bridge `--skip-git-repo-check` override so detached run-scoped
worktrees under the coordination directory do not fail Codex trusted-directory
checks during baseline, parse, apply, bench, or evaluate turns. If perf recovery
detects that the disposable workspace is no longer a valid git worktree, it
rebuilds that run-scoped worktree before attempting the rejection cleanup. The
disposable worktree is removed when the run is stopped or becomes terminal.

Autonomous worker `runsc` mounts now stage Codex home state through a
coordination-owned sidecar directory under
`~/.rust-chatbot/coordination/runsc-sidecars/codex-home/` instead of bind-mounting
`config.toml` and `auth.json` as separate files into `CONTAINER_HOME/.codex`.
The launcher seeds missing sidecar files from the shared `~/.rust-chatbot/.codex/`
config root, with legacy repo-local config still accepted as a fallback during
the split migration, then mounts that single writable `.codex` directory into
the container. Gemini and Qwen sidecars now resolve from that same shared config
root as well. This keeps the mount surface limited to provider home state while
still allowing temp-file-plus-rename config persistence paths to run within one
writable directory.

Autonomous provider execution now emits a positive `task_runsc_ready` event
after the shared `runsc` workspace and environment are activated successfully.
The Recent Events pane therefore shows an explicit "Runsc started correctly..."
marker with the shared workspace root and explicit include list for that task.
Survey workspace artifacts also render their summaries as `Surveyed workspace
via runsc ...` when they were collected through the sandboxed survey path.

## Artifacts And Visibility

Completed tasks publish artifacts into `artifacts/<task_id>/`, and the UI reads
events plus task snapshots directly from disk. The desktop UI is a control and
inspection surface, but disk state remains authoritative.

Run summary rows are also refreshed eagerly when task completion, retry, or
failure is recorded. That keeps the list-view status and `updated_at`
timestamp aligned with the actual task event time instead of waiting for a
later detail-view snapshot load.

For autonomous `runsc` writes, the authoritative writeback path is:

1. the provider mutates the shared sandbox workspace
2. Rust derives changed outputs from the declared `write_scope` plus the
   before/after filesystem state and snapshots those files into
   `artifacts/<task_id>/materialized-files.json`
3. Rust writes those same captured contents back into the host workspace

That keeps host writes explicit and artifact-backed instead of relying on the
container wrapper to sync whole directory trees back after each provider call.
Single-file prompt and document writes now validate their artifact body against
the captured materialized file contents when available, so heading-light
continuous narratives are accepted as long as the recorded body matches the
actual written document text.

The run-detail "Recent Events" panel reads the newest 40 entries from
`events.jsonl`. The event loader trims from the head of the file and preserves
the remaining file order, so the UI can reverse that tail slice for display
without dropping later events on long-running runs. Task status counts are
derived separately from the current task snapshots, so event recency and
task-state freshness are related but not loaded through the same code path. A
`Copy` button now serializes that exact visible 40-event slice into the same
line format shown in the panel and places the full text on the system
clipboard.

Clicking a run in the right-side Runs list also copies that run's persisted
configuration into the left-side create form so operators can use an existing
run as a template for a new one. For perf runs, the form strips runtime-only
note keys such as `perf_worktree_root` and `perf_worktree_path` back out of the
editable Notes field while restoring the original `bench_script_path` into its
dedicated input.

The run-detail "Restart Failed" control is a popup-driven flow. Before the
coordinator requeues failed tasks, the operator can pick one task, override its
`assigned_provider`, and, if that task is currently `Completed`, mark it failed
so the subsequent restart includes it.

Stopping a run does not rewrite the persisted `auto_retry_failed_tasks`
preference. If the operator later uses "Restart Failed", the run resumes with
the same retry policy it had before the stop.

Run stop now also drains live worker processes from the claim ledger before it
returns. The coordinator gathers every worker PID represented by the run's
claim files, including already-adopted provider workers, and escalates from
`SIGTERM` to `SIGKILL` when a worker keeps heartbeating after the stop request.

Stub-marker validation still applies to ordinary workspace writes, but the
final `CodeReview` markdown materialization step is exempt from that specific
check. Review documents can legitimately mention phrases such as `todo!` or
`not yet implemented` while describing findings, and that prose should not
cause the document task to fail and loop on restart.

For Rust source files, the shared stub-marker scan now ignores string literals
before checking for blocked markers. That keeps real executable `todo!()` /
`unimplemented!()` code blocked while allowing test fixtures and assertions to
mention those tokens inside quoted strings.

Perf baseline, bench, and evaluate tasks also report their `/tmp/perf_*.json`
outputs as `changed_files`, so acceptance validation treats those temp
artifacts as the concrete write effects for the perf workflow.

The durable run layout is rooted at:

- `<shared_infra_dir>/coordination/<run_id>/run.json`
- `<shared_infra_dir>/coordination/<run_id>/events.jsonl`
- `<shared_infra_dir>/coordination/<run_id>/tasks/<task_id>.json`
- `<shared_infra_dir>/coordination/<run_id>/claims/<task_id>.json`
- `<shared_infra_dir>/coordination/<run_id>/artifacts/<task_id>/result.json`
- `<shared_infra_dir>/coordination/<run_id>/artifacts/<task_id>/notes.md`
- `<shared_infra_dir>/coordination/<run_id>/artifacts/<task_id>/materialized-files.json`
- `<shared_infra_dir>/coordination/<run_id>/artifacts/<task_id>/review-metadata.json`
- `<shared_infra_dir>/coordination/<run_id>/runsc-shared/workspace/`

In the default local setup, `<shared_infra_dir>` resolves to
`~/.rust-chatbot`, so a run's artifacts are typically written under
`~/.rust-chatbot/coordination/<run_id>/artifacts/<task_id>/`.

## Task Rewiring

When a plan task generates child tasks (e.g., `GeneratedWriteTask`, `GeneratedReadTask`),
the `rewire_follow_up_tasks(...)` function updates dependent tasks to depend on the
newly generated tasks instead of the plan task directly.

Key rules for rewiring:

- Generated children (tasks with `parent_task_id = source_task_id` and kind
  `GeneratedReadTask`/`GeneratedWriteTask`) are skipped — they already have the
  correct parent relationship.
- `ReviewArtifacts` tasks that appear in the `generated_ids` list (review tasks
  spawned by the plan) are also skipped — they keep their original dependencies.
- `ReviewArtifacts` and `MaterializeDesignDoc` tasks that depend on the source
  task but are NOT generated children are rewired to depend on the generated tasks.
- `ApplyApprovedEdits` tasks are marked `Blocked` but not rewired.

## PerfHillClimber Mode

`PerfHillClimber` seeds two parallel initial tasks (`PerfBaseline` + `PerfParse`)
and then drives all subsequent work through expansion hooks rather than pre-seeding
the full DAG. See the [Overview workflow modes section](overview.md#workflow-modes)
for the complete flow and the hill-climbing chain description. Bench/link
failures are recorded into the bench JSON artifact and passed through to
`PerfEvaluate`, which rejects the idea, resets tracked plus untracked worktree
state, and continues to the next idea.

## Related Docs

- [Overview](overview.md)
- [Autonomous Design](../autonomous/design.md)
- [Coordination Notes](../autonomous/coordination.md)
- [Boundary](../boundary.md)
