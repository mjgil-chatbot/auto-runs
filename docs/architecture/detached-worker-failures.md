# Detached Worker Failure Handling

Detached provider workers are launched by the background driver but execute in a
separate `auto-runs-worker` process. Two failure modes matter before normal
task execution starts:

- the worker can fail during bootstrap before it adopts the claim
- the worker can exit nonzero after spawn but before the lease naturally expires

The current implementation hardens both cases:

- `auto-runs-worker` initializes its own tracing stack exactly once through
  `rust_debug::init_tracing("auto-runs-worker")`
- tracing initialization is non-fatal: startup prints any initialization error
  or panic to stderr and continues instead of aborting with exit code `101`
- detached worker stderr is written under `coordination/<run_id>/worker-logs/`
  so panic text survives even though the worker is backgrounded
- when the launcher observes a spawned worker exit nonzero, it records
  `task_worker_exited`, includes the stderr log path and stderr tail in the
  event detail when available, and immediately requeues or fails the task
  instead of waiting for lease expiry

This keeps worker failures diagnosable from the durable run spool and prevents
crashed background workers from holding a claim for the full lease window.
