# Driver Service Binary Refresh

The background `drive-forever` loop runs under the OS-level service managed by
`scripts/driver-service.sh`. That means reinstalling the repo binaries does not
automatically replace the already-running service process: systemd or launchd
keeps the old executable image alive until the service is restarted.

To keep the service consistent with the installed binary:

- `install_auto_runs_binaries()` fingerprints the installed `auto-runs` binary
  before and after `cargo install --force`.
- The helper sets `AUTO_RUNS_BIN_CHANGED=1` only when the installed service
  binary content actually changed.
- `ensure_driver_service_running()` restarts the driver service when it is
  already running and `AUTO_RUNS_BIN_CHANGED=1`.
- If the service is not running, the helper still just starts it; no extra
  restart is needed.

This keeps `just start` and `just restart` aligned with the currently installed
`auto-runs` binary without forcing a service bounce when the install produced
the same executable bytes.

On Linux, that service restart path must not tear down already-adopted
`auto-runs-worker` children. The generated systemd unit therefore uses
`KillMode=process` and `SendSIGKILL=no` so a driver restart replaces only the
`drive-forever` process and leaves detached worker-owned task claims alive.
The unit also uses `Restart=always` rather than `Restart=on-failure`: in
practice, the driver can be terminated by a plain `SIGTERM`, and systemd treats
that as a clean exit for this service. With `Restart=on-failure`, the service
would stay `inactive (dead)` and queued runs would stop progressing until
someone restarted it manually.
The same helper now also refreshes the installed service definition when the
generated unit or plist content drifts, so shell-only service fixes propagate
on the next `just start` or `just restart` even when the Rust binary bytes did
not change.
