# Boundary

This document defines the ownership boundary between `rust-chatbot` and
`auto-runs`, and the allowed import path if autonomous coordination is brought
back into `rust-chatbot` later.

## Ownership

`auto-runs` owns:
- Autonomous coordination engine
- Council runtime
- Autonomous code-review workflows
- Autonomous workflow registry
- Autonomous runsc image/script assets
- Autonomous design and coordination docs
- Standalone desktop app for creating, driving, and inspecting runs

`rust-chatbot` owns:
- Normal chat sessions and chat UI
- Provider chat execution paths
- Session persistence and chat-side MCP services
- Non-autonomous worker flows
- General app sync/state that is unrelated to autonomous coordination

## Current Surface

`rust-chatbot` no longer exposes operational autonomous entry points.
- No autonomous launch surface in the desktop UI
- No autonomous background driver
- No autonomous worker mode
- No `chatbot-ctl` run-management surface

`auto-runs` is now the operational home for those flows.

## Shared Contracts

Cross-repo shared types belong in `crates/auto-runs-contracts`.

Current shared contract:
- `AgentRole`

Future shared contracts should also move there when they are needed by both
repos, for example:
- Run/task/event wire types
- Query/request DTOs
- Stable disk-format structs

Rules:
- If a type is needed in both repos, move it to `auto-runs-contracts`.
- Do not duplicate the same contract type in both repos.
- Do not make one repo import private source files from the other repo.

## Allowed Import Path

If autonomous coordination is imported back into `rust-chatbot` later, the
import must be explicit and narrow:

1. `rust-chatbot` may depend on `auto-runs-contracts` for shared DTOs.
2. `rust-chatbot` may depend on a future public `auto-runs` library API.
3. `rust-chatbot` must not use source-level coupling such as:
   - cross-repo `#[path = ...]`
   - symlinked Rust modules as a permanent boundary
   - importing internal `auto-runs` module files directly

That means the clean future shape is:
- `rust-chatbot`: chat app
- `auto-runs`: autonomous app + public library
- `auto-runs-contracts`: stable shared boundary

## Migration Rule

When removing the remaining dormant compatibility references from
`rust-chatbot`, follow this order:

1. Move any reused stable type into `auto-runs-contracts`.
2. Replace both repos to use the shared contract.
3. Remove the old `rust-chatbot` autonomous compatibility code.
4. Delete temporary compatibility mechanisms once both repos build cleanly.

This keeps the cutover incremental without reintroducing source coupling.
