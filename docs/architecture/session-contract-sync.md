# Session Contract Sync

`auto-runs` synthesizes `ChatSession` records for two detached-provider paths:

- council turns in `src/coordination/council/session.rs`
- task execution sessions in `src/coordination/parts/artifact_helpers.rs`

Those constructors now initialize the full shared `chatbot-worker` session
contract, including `runsc_image`. Keeping those synthetic sessions in lockstep
with the shared struct prevents sibling schema additions from breaking
`auto-runs` release builds when the detached worker surface evolves.
