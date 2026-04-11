# Council Driver Recovery

The background driver now treats run execution as fault-isolated work instead of
an all-or-nothing polling pass.

## Driver Behavior

- `drive_active_runs_once()` logs per-run failures and continues polling the
  remaining active runs.
- A malformed council run can no longer block unrelated queued or heartbeating
  work just because it appears earlier in the authoritative run scan.

## Missing Council State

Freeform Council runs now distinguish between two missing `council.json`
states:

- Pristine recoverable state: if the workspace still exists and there is no
  ledger, no pending commit, no events, and no survey artifact, the runtime
  reseeds `council.json` and records `council_state_recovered`.
- Corrupt unrecoverable state: if there is evidence of prior state or the
  workspace is already gone, the runtime synthesizes a failed council state,
  marks the run `failed`, and records `council_state_corrupt`.

This keeps malformed council spools from poisoning the background driver while
still preserving a durable forensic trail in the run events.
