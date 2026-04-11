# GitHub Branch Fix

`GithubBranchFix` is a dedicated single-worker workflow for monitoring and
repairing GitHub Actions failures on one branch. It keeps the lightweight
plain-text worker execution model and automatic git publish path, but it no
longer reuses the lightweight task-list-document configuration.

## Inputs

The workflow uses one run-level worker model from `run.provider` plus two
note-backed settings:

- `github_branch_fix_branch`
- `github_branch_fix_poll_minutes`

The desktop run form does not expose `completion_criteria` for this workflow.
That field remains internal coordination state and branch-fix runs submit an
empty user-facing completion-criteria value.

`max_agents` is forced to `1` for this workflow at run creation and update
time, so the branch-fix loop always executes with one worker model and one
active claim at a time.

The input `workspace_path` may point either at the primary git checkout root
for the repo under repair or at an already-created linked worktree that is
already checked out to the requested branch. When the input is the primary
checkout root, the coordinator immediately rewrites the run into a
branch-specific linked worktree at
`{workspace_path}-{sanitized-branch-name}-{run-id-fragment}` so concurrent
branch-fix runs on the same branch do not share one mutable worktree. When the
input is already a linked worktree for the requested branch, the coordinator
keeps that directory as-is and does not synthesize another sibling worktree. In
both cases it stores these note-backed paths:

- `github_branch_fix_source_workspace`
- `github_branch_fix_worktree_path`

All seeded tasks run inside the selected linked worktree from the first survey
pass forward. If the worktree directory disappears before a retry or manual
restart, the coordinator recreates it at the same persisted worktree path and
reuses the requested branch.

When workspace prep needs a `github` remote alias for publish or GitHub CLI
operations, it first inspects existing git remotes such as `origin` or
`upstream` and reuses any GitHub-hosted repository URL it finds there. This
keeps linked worktree names like `rustc-wasm-builder-enscripten` from being
mistaken for the actual GitHub repo slug. Only workspaces without an existing
GitHub remote fall back to `gh repo create` or `gh repo view`.

Generated polling read tasks also have workflow-specific timeout handling: if a
GitHub Actions poll task hits its provider time budget, the coordinator
requeues it immediately and restores the consumed attempt so the branch-fix
loop keeps polling instead of surfacing a terminal task failure.

Generated write tasks also have workflow-specific git-publish recovery. If the
runtime stages the worker's local changes but `git commit` or `git push` fails,
the task is requeued instead of being marked terminally failed while attempt
budget remains. The coordinator appends the git failure details directly to the
same task prompt so the next worker turn starts from the existing worktree
state, fixes only the publish blocker, reruns the relevant local checks, and
lets the runtime retry the automatic commit or push. If the previous attempt
already created the local commit and only the push remained, the retry path can
resume by pushing the existing `HEAD` without forcing unrelated code churn.

## Seeded Flow

Each run seeds its own four-task branch-fix loop:

1. `SurveyWorkspace`
2. implementation `PlanFlatTasks`
3. validation `PlanFlatTasks`
4. validation `ReviewArtifacts`

All three provider-backed tasks use the same single worker model from
`run.provider`.

## Planner Contract

Both branch-fix `PlanFlatTasks` planners use the lightweight plain-array plan
format instead of the typed `plan_tasks` envelope. The runtime does not apply
JSON-schema structured-output mode or in-process contract reasks to these
planner tasks, because their prompts already ask for a raw JSON array and the
artifact parser accepts that array directly.

The implementation planner bootstraps the branch before steady-state repair
work starts:

- create the canonical design doc if it is missing
- inspect `.github/workflows/` and add or update branch coverage when the
  target branch does not already trigger tests
- ensure a versioned pre-commit hook exists and runs the repo test suite before
  commits
- emit worker tasks that poll GitHub Actions for the configured branch at the
  configured minute interval
- require fixing tasks to inspect the failing run directly, add or update
  tests, commit, push, and verify a newer GitHub Actions run completed
  successfully after the push

Generated write tasks still use their `title` as the exact automatic commit
message. Every branch-fix prompt now also carries an explicit GitHub CLI auth
preflight: workers must verify `gh auth status` before any `gh run list` or
other API-backed GitHub CLI call, prefer authenticated `GH_TOKEN`/`GITHUB_TOKEN`
or stored `gh auth login` state, and treat HTTP 403 responses as an
auth/rate-limit check rather than assuming the repository itself is blocked.
When supported, `gh auth status --active` is preferred, but older `gh` builds
fall back to plain `gh auth status`. The runtime also enforces that
precondition at the input rail for branch-fix generated read/write tasks and
the final review task, so a run fails fast with an auth-specific error before
provider execution if the CLI auth check is not healthy.

## Validation Contract

Validation remains a follow-up review loop, but it is stricter about terminal
success. The read-only validation planner also emits a plain JSON array of task
objects, while the follow-up review still uses the typed `review_findings`
contract. That review may only declare completion once it can cite evidence
that:

- the target branch has GitHub Actions workflow coverage
- the repo has a pre-commit hook that runs tests
- the latest fix work preserved or expanded relevant tests
- a newer GitHub Actions run completed successfully for the target branch after
  the latest fix push in the current pass

If any of those checks fail, the review emits smaller follow-up fix tasks
directly from the current workspace state. There is no task-list document and
no splitter or validator model override in this workflow.

When a branch-fix review ends `partial` without emitting any explicit follow-up
tasks, the coordinator now synthesizes a read-only wait task that keeps polling
GitHub Actions for the target branch until a completed successful run exists
for the latest fix push. That synthesized wait task is then wired into another
follow-up review so the workflow stays active instead of terminally failing
while CI is merely still in progress.

Those wait tasks now use explicit response markers. `WAITING_FOR_SUCCESS:`
means the newest matching run still has not completed successfully, so the
runtime requeues the same wait task without consuming attempt budget.
`SUCCESS_CONFIRMED:` means a completed successful run exists for the latest
fix commit, so the follow-up review may proceed.

The same live success gate now also applies to generated branch-fix validation
reads whose job is to verify or validate a successful post-push Actions run.
Even if a worker mistakenly reports `COMPLETED`, the runtime checks `gh run
list` against the current `HEAD` and requeues that validation read instead of
letting the final review close the run while CI is still only `in_progress`.

Branch-fix validation planning can still end its own artifact with `partial`
after it has emitted child validation tasks. That planner artifact no longer
fails the whole run once those child tasks and the terminal review complete;
the run-level completion gate treats the planner as superseded by the spawned
validation flow.

The same completion gate now also ignores `partial` outcomes from branch-fix
generated write tasks once a later review task depends on them. Those write
tasks can finish local code and push successfully before the newer GitHub
Actions run has completed; final CI success remains the responsibility of the
follow-up validation review rather than the write task itself.
