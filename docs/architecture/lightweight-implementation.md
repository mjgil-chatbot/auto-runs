# Lightweight Implementation

`LightweightImplementation` is a note-configured autonomous workflow with its
own lightweight runtime policy. It is intentionally looser than the standard
implementation pipeline: coordination lives primarily in a mutable task-list
document, configured providers are treated as authoritative, and end-of-pass
validation drives the loop instead of per-task artifact acceptance.

`GithubBranchFix` now has its own workflow contract and UI surface. It still
reuses the lightweight plain-text worker execution path, but it no longer
shares the task-list-document, splitter, validator, or multi-worker
configuration described in this document. It also does not use lightweight's
missing-directory bootstrap: branch-fix runs require a primary git checkout as
input and immediately switch into a run-specific linked worktree named from
that checkout path, the target branch, and a run-id fragment.

## Inputs

The workflow consumes:

- the implementation idea from `run.goal`
- workspace state from the selected run workspace
- optional canonical design doc and active task-list targets from run config

The UI persists four note-backed settings on the run:

- `lightweight_task_splitter`
- `lightweight_validation_model`
- `lightweight_worker_models`
- `lightweight_task_doc`
- `lightweight_repo_configuration`

In the create-run UI, `lightweight_worker_models` is edited through repeatable
dropdown rows with inline add/remove controls rather than freeform text entry.
The optional design-doc and task-list bootstrap targets remain plain text
fields with no forced placeholder value in the form. `lightweight_repo_configuration`
is currently a constrained dropdown with `N/A`, `Rust`, and `Zig`. The lightweight
`Max Agents` control renders directly below the worker-model rows so the pool
size stays grouped with the worker assignments it governs.

The current task-list pass counter is also note-backed as
`lightweight_task_doc_cycle`.

## Seeded Flow

Each run seeds exactly four tasks:

1. `SurveyWorkspace`
2. implementation `PlanFlatTasks`
3. validation `PlanFlatTasks`
4. validation `ReviewArtifacts`

The implementation planner is assigned to `lightweight_task_splitter`. The
validation planner and final validation review are assigned to
`lightweight_validation_model`, which defaults to the splitter model when the
note is absent on older runs.

If the canonical design doc or active task-list document does not exist yet,
the implementation planner bootstraps them first from the goal plus workspace
survey, then emits the first pass of implementation fragments.

## Expansion Model

The implementation planner emits small generated tasks for the worker pool.
Those generated tasks are assigned round-robin across
`lightweight_worker_models`, and claim routing does not reinterpret those
assignments through the provider capability registry. If a lightweight task is
configured for `minimax`, the runtime keeps that assignment instead of
rerouting to a provider that merely advertises stronger write categories.
When a fragment depends on another generated task, the planner can declare that
edge through `depends_on_titles`; the runtime resolves those title references
into durable task-ID dependencies before workers begin claiming tasks. Each
generated task also carries an `estimated_minutes` budget; if a worker runs
past that estimate, the runtime stops that attempt and requeues the task for
another worker while attempt budget remains.

When implementation tasks are expanded, the downstream validation planner is
rewired to depend on them. That makes validation planning wait for the current
implementation pass instead of running immediately after the planner artifact.

The validation planner emits read-only generated tasks that inspect the current
workspace and artifacts. Once those validation tasks are expanded, the final
validation review is rewired to depend on them.

## Review Loop

The final `ReviewArtifacts` task is the pass gate for each pass.

If validation finds more work:

- it writes the next task-list document
- `complete_task(...)` advances `lightweight_task_doc` to that new path
- the review emits follow-up generated fix tasks
- those fix tasks are assigned back onto the worker pool
- the workflow spawns a follow-up `ReviewArtifacts` task that reads the updated
  current task-list path from `run.notes`
- each lightweight follow-up review recomputes its writable coordination scope
  from that updated note-backed path, so it can update the live current-pass
  task list and create the next-pass task list without inheriting stale
  pass-N-1 write permissions

If validation emits no follow-up fix tasks, the run completes on the current
validated task list.

Run completion treats the terminal lightweight review as authoritative. Earlier
review passes that ended `partial` and already spawned a follow-up review are
considered superseded loop state and do not fail the run-level completion gate.

## Coordination Contract

This workflow uses two different provider-boundary contracts:

- planning and validation review tasks use machine-readable JSON; in lightweight
  mode the runtime accepts plain JSON arrays without the typed envelope wrapper,
  to avoid validation errors when providers return loose JSON
- generated implementation and validation worker tasks use lightweight plain
  text completion summaries plus snapshot-derived workspace state instead of
  standard task reports

Worker prompts explicitly remind implementation tasks to update the active
task-list document as they complete their fragment. The task planner (not the
worker) sets each generated write task's `title` to the exact git commit message
the coordinator will use when automatically committing and pushing after
completion. The worker receives no instruction about git commit messages; it
only completes the assigned slice and reports a plain-text status. The coordinator
owns the commit/push flow. Multiline titles are preserved as multiline commit
messages. Validation prompts
explicitly remind review tasks to either advance the task-list document or
declare the implementation complete.

Generated task scopes are intentionally repo-wide in lightweight mode. Planner
and review-emitted write tasks set both `read_scope` and `write_scope` to
`["**"]`, while read-only tasks use `read_scope=["**"]` and leave
`write_scope=[]`. The runtime relies on patch-level git publish, not narrow
declared file ownership, to reconcile concurrent edits onto the latest branch
head.

Lightweight-specific execution rules:

- configured providers are used as-is; lightweight claim routing never falls
  back through registry routing
- typed planner/review payload validation now normalizes generated task scopes
  that come back as absolute paths under the active workspace root into
  repo-relative paths before persistence; absolute scopes outside the workspace
  still fail validation and trigger the normal reask path
- the active lightweight task-list note is normalized back to a workspace-
  relative path when it points inside the current workspace root, so prompts
  and generated task scopes stay on the same relative-path contract even if an
  older run stored that note as an absolute host path
- if a lightweight run creates a missing workspace directory, the runtime also
  initializes a local git repository there immediately, wires a local bare
  `origin` remote under the coordination data root, and attempts to create or
  attach a GitHub remote named `github` via the local `gh` CLI so later
  automatic commit/push steps have both a deterministic local publish target
  and a GitHub mirror for freshly bootstrapped workspaces
- lightweight workspace preparation does not stop at "directory exists": if an
  existing lightweight workspace is missing `.git`, the runtime initializes it
  in place before the task runs, and if the local `origin` or `github` remotes
  are missing it attaches the managed remotes needed by the lightweight git
  publish flow
- when `lightweight_repo_configuration=rust`, every lightweight workspace
  preparation pass recopies `.gitignore`, `.cargo/config.toml`,
  `rust-toolchain.toml`, `rustfmt.toml`, `.githooks/pre-commit`, and
  `scripts/install-git-hooks.sh` from `$AUTO_RUNS_RUST_SCAFFOLD_ROOT`,
  overwriting drift in those repo-managed files, marking the copied scripts
  executable, and rerunning the copied hook installer so both fresh and
  pre-existing workspaces converge back onto the scaffold's Rust toolchain and
  git-hook setup before task execution
- when `lightweight_repo_configuration=zig`, every lightweight workspace
  preparation pass rewrites the repo-managed `.gitignore`,
  `.githooks/pre-commit`, and `scripts/install-git-hooks.sh` files to the
  built-in Zig hook profile, marks the scripts executable, and reruns the hook
  installer so both fresh and pre-existing workspaces converge onto a
  Zig-oriented `zig fmt --check .` plus `zig build test` pre-commit contract
- if a configured lightweight splitter or worker provider is unavailable, the
  affected task fails immediately with no provider fallback or automatic retry;
  lightweight runs treat configured provider assignment as authoritative, and
  the queued-task claim path now snapshots that assigned provider and any
  cooldown state before it mutates task state into the shared lightweight
  failure helper
- lightweight worker claims may overlap on the active task-list document, so
  multiple workers can update that coordination file concurrently; claim
  blocking still applies when their non-coordination write scopes overlap, and
  workers still wait for declared generated-task dependencies
- lightweight completions do not run the normal artifact acceptance rails
- lightweight generated write completions use **patch-level publish** instead of
  staging worker files directly; see the Publish Pipeline section below
- lightweight review passes do not open `review_approval` interrupts; if
  validation emits follow-up fix tasks, they queue immediately and the loop
  continues without a policy pause
- lightweight report parsing does not require `changed_files`; generated-write
  materialized-file snapshots are retained while precise publication is pending
  and removed after successful publication.
  Other lightweight reports keep their compact execution summaries
- task-list advancement is driven by the validation review emitting follow-up
  tasks, not by file-tracking evidence

## Publish Pipeline

When a lightweight generated write task completes, the coordinator publishes
its changes through a patch-level pipeline rather than staging the worker's
files directly. This enables safe concurrent edits to the same file.

### Why not direct staging?

Direct staging (git add -- <paths>) would overwrite concurrent changes. If
Task A modified `src/lib.rs` based on an older branch head, and Task B modified
the same file on a newer head, staging A's version onto B's workspace would
discard B's work.

### The patch approach

Instead, the coordinator builds a **patch series** from the worker's materialized
files — one patch per file containing only the actual diff hunks that changed.
Then it applies those patches onto the **latest branch head** using three-way
merge, retries on push races, and defers only true conflicts.

### Pipeline steps

If the workspace repo has no `HEAD` yet, the coordinator first creates the
initial commit directly in the workspace from the worker's materialized files
and pushes `origin` plus the optional `github` remote. After that bootstrap
commit exists, later lightweight publishes switch to the patch flow below.

```
1. Build patch series
   ├── capture_materialized_files_from_snapshot()
   ├── for each changed file: compute diff hunks (preimage → postimage)
   └── result: PatchSeries { patches: [PatchFile { path, hunks: [PatchHunk] }] }

2. Fetch latest branch head from origin
   └── git rev-parse origin/main (or configured target ref)

3. Clone to clean temp worktree at latest head
   └── git clone --depth 1 <remote> <temp-worktree>

4. Apply patches with three-way merge
   └── git apply --3way --index for each patch
       - git uses base blob to merge worker hunk with concurrent branch changes
       - non-overlapping line edits auto-merge cleanly
       - same-hunk conflicts are detected per-hunk

5. Commit result with planner-authored title
   └── git commit -m <task.title>

6. Push with retry on race
   └── git push origin HEAD:refs/heads/main
       - if non-fast-forward (another task pushed): retry from step 2
       - max 5 retry attempts

7. Handle outcomes
   ├── Published { commit } — success
   ├── NoChange — task produced no net changes
   ├── AlreadyApplied — delta was already in branch
   └── DeferredForResolution { conflicts } — true conflict requires resolution task
```

### PublishOutcome enum

```rust
pub enum PublishOutcome {
    Published { commit: String },       // git hash of published commit
    NoChange,                            // task made no net edits
    AlreadyApplied,                      // delta was already present
    DeferredForResolution {
        conflicts: Vec<PatchConflict>,   // true per-hunk conflicts
    },
}

pub struct PatchConflict {
    pub path: PathBuf,
    pub hunk_index: usize,
    pub base_oid: String,
    pub their_oid: String,
}
```

### Concurrent same-file edits

Two tasks editing different lines of the same file:

```
Base: src/lib.rs has lines 1 and 20
Task A modifies line 1, Task B modifies line 20
```

1. Task A publishes first → its patch applies cleanly onto current head
2. Task B publishes → fetches new latest (now includes A's commit)
   → applies B's patch using three-way merge
   → B's line-20 change merges cleanly with A's line-1 change
3. Final commit B's diff contains only B's line-20 change

Result: clean parallel commits without overwrite.

### True conflicts

Only same-hunk incompatibilities cause `DeferredForResolution`:

```
Task A changes function signature one way
Task B changes same function signature differently
```

No auto-merge is possible — this is a semantic conflict requiring resolution.
The original worker task still succeeded in producing a delta; the resolution
task handles merging the incompatible hunks.

### What the worker does

The worker has no knowledge of git, patches, or commits:

1. Reads the generated task prompt (read_scope/write_scope/allowed_tools)
2. Modifies only files within declared write_scope
3. Returns a plain-text status line:

```
COMPLETED:  — finished slice, workspace changed
NO_CHANGE:  — slice already satisfied, no changes
PARTIAL:    — made progress but more remains
BLOCKED:    — could not proceed due to blocker
FAILED:     — unexpected failure
```

The declared `write_scope` still constrains what the worker is supposed to
edit, but in lightweight mode it no longer acts as a claim-time mutex between
generated write tasks. Concurrent lightweight writes are reconciled later by
the patch-level publish pipeline.

4. The coordinator captures the materialized files from the worker's snapshot
5. The coordinator builds and applies patches independently

The worker never:
- runs git commands
- authors commit messages
- pushes changes
- knows about the publish pipeline
