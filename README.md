# auto-runs

[![License: 0BSD](https://img.shields.io/badge/License-0BSD-blue.svg)](LICENSE)

Standalone autonomous coordination app with an egui desktop UI.

## Overview

auto-runs provides an autonomous AI agent coordination engine with:

- **Coordination engine** — drives multi-agent council workflows with turn policies,
  iteration limits, and review gates
- **Council runtime** — orchestrates role-based agent councils (e.g. coder, reviewer,
  summoner) with configurable turn policies
- **Autonomous code review** — review helpers that diff against a base, scope paths,
  and apply fixes automatically or with manual approval
- **Provider workflow registry** — registers and dispatches work across multiple AI
  provider backends
- **runsc support assets** — Docker image and scripts for the autonomous runtime
  container used to execute agent tasks

## Sibling Dependencies

auto-runs depends on sibling repos at the same parent directory level:

```
parent/
├── auto-runs/        ← this repo
├── chatbot-worker/   ← shared chatbot_lib runtime
├── ai-providers/     ← AI provider backends
└── the-markdown-commons/ ← shared markdown utilities
```

Clone or symlink these sibling repos into place before building.

## Prerequisites

- **Rust** 1.95.0 (the release verification toolchain)
- **Linux x86_64 builds:** `clang` and `mold`, as selected by `.cargo/config.toml`
- **Docker** (for building the `autonomous-runsc` container image used to execute
  agent tasks)
- **Node.js 22+** (for the runsc container's bundled AI provider CLIs)

## Building

```bash
# Build the release binary
cargo +1.95.0 build --release

# Or use the "ship" profile (full LTO, no incremental)
cargo +1.95.0 build --profile ship
```

The binary ends up at `target/release/auto-runs` (or `target/ship/auto-runs`).

## Running

```bash
# Launch the desktop UI
./target/release/auto-runs
```

The UI will prompt for a workspace path, design doc, and workflow configuration.
Use the **Run** button to dispatch a coordination run.

## Driver Service

`auto-runs` includes a background driver loop so active runs can continue
progressing even when the desktop UI is closed. The UI does not install,
start, or supervise that driver internally. Service management is handled by
repo scripts, and the app-launch scripts ensure the service is running before
they bring up the UI.

The installed service remains OS-native:

- Linux: `~/.config/systemd/user/auto-runs-driver.service`
- macOS: `~/Library/LaunchAgents/com.mjgil.auto-runs-driver.plist`

```bash
# Install or refresh the user service
./scripts/driver-service.sh install

# Manual lifecycle commands
./scripts/driver-service.sh start
./scripts/driver-service.sh stop
./scripts/driver-service.sh restart

# Inspect the service
./scripts/driver-service.sh status
./scripts/driver-service.sh logs

# Remove the service
./scripts/driver-service.sh uninstall
```

For day-to-day repo operations, the root `justfile` exposes:

```bash
just install                 # install/rebuild auto-runs and auto-runs-worker
just start                   # install/relaunch the UI app
just restart                 # reinstall + restart the UI app
just start-driver-service
just stop-driver-service
just restart-driver-service
just status-driver-service
just logs-driver-service
```

`just install` refreshes only the installed binaries. `just start`,
`just restart`, and `./scripts/launch.sh` all ensure the driver service is
installed and running before the UI process starts.

`just start` and `just restart` reinstall the binaries first. If that install
changes the `auto-runs` service binary, they also restart the driver service so
`drive-forever` does not keep running the previous executable.

## Docker Image (runsc)

The `autonomous-runsc` Docker image bundles AI provider CLIs for executing agent
tasks in isolation. Build it with:

```bash
./scripts/build_autonomous_runsc_image.sh
# Override tag:
./scripts/build_autonomous_runsc_image.sh my-registry/autonomous-runsc:v1
```

The image includes:
- Claude Code (`@anthropic-ai/claude-code`)
- Codex (`@openai/codex`)
- Gemini CLI (`@google/gemini-cli`)
- Qwen Code (`@qwen-code/qwen-code`)

## Project Structure

```
auto-runs/
├── src/
│   ├── main.rs          # eframe/egui desktop app entry point
│   ├── lib.rs           # App library plus selected shared-runtime re-exports
│   ├── coordination/    # Core coordination engine
│   ├── code_review/     # Autonomous review helpers
│   └── workflow.rs      # Workflow registry
├── crates/
│   └── auto-runs-contracts/  # Types shared with rust-chatbot
├── docker/
│   └── autonomous-runsc.Dockerfile
├── scripts/
│   └── build_autonomous_runsc_image.sh
└── docs/
    └── boundary.md      # Repo boundary rules
```

## Repo Boundary

`rust-chatbot` owns the chat session/app layer. `chatbot-worker` owns the
shared worker/runtime surface. `auto-runs` owns autonomous coordination.
Shared wire types belong in `crates/auto-runs-contracts`. See
[`docs/boundary.md`](docs/boundary.md) for the full rules on cross-repo
imports.

## Profiles

| Profile    | Use case                          |
| ---------- | --------------------------------- |
| `dev`      | Default debug build               |
| `release`  | Fast release build (opt-level 1)  |
| `ship`     | Full production build (opt-level 3, LTO) |
| `coverage` | Test coverage instrumentation     |

## Release verification

The root package, `crates/auto-runs-contracts`, and `proof/aeneas-model`
are separate Cargo packages. Run release check, build, and tests for each
manifest; root workspace tests do not execute dependency-package tests.
For example, start with `cargo +1.95.0 test --release --all-features --bins --tests`.
Use `--manifest-path` to select each additional package.

Default scaffold lookup uses `~/git/rust-project-scaffold` and
`~/git/zig-project-scaffold` for the current user. Scaffold roots can be overridden with `AUTO_RUNS_RUST_SCAFFOLD_ROOT` and
`AUTO_RUNS_ZIG_SCAFFOLD_ROOT`. The companion repositories must be placed
beside this repository. Runtime provider CLIs, credentials, Docker/runsc,
and external scaffold/extraction tools are configured separately.
The release report records which native checks actually ran; Rust checks
alone do not certify the optional Lean/Aeneas packages or live agent sessions.
