# Review profile

Review the requested scope against the current implementation in
`docs/architecture/overview.md`, `docs/architecture/autonomous-coordination.md`,
and the ownership boundary in `docs/boundary.md`.

- Check durable task, claim, driver, and artifact state together. Recovery must
  preserve lease ownership, dependency ordering, and bounded retry policies.
- Keep workflow-specific policy separate: lightweight implementation, GitHub
  branch repair, and council runs have different completion and recovery rules.
- Check publication against the actual remote base. Preserve unrelated staged,
  unstaged, and untracked workspace files; defer unrecognized remote conflicts.
- Treat provider responses as untrusted input. Validate structured artifacts,
  write scopes, execution permissions, and declared outcomes before completion.
- Keep credentials out of logs and artifacts. Resolve user paths at runtime and
  make tests independent of installed providers, scaffolds, and credentials.
- Preserve the public contracts crate boundary. Do not import sibling private
  modules or add implicit machine-specific build configuration.

Run the release checks documented in the README. Report the command, result,
and any untested platform or external-service behavior. Inspect regression tests
for changed behavior; add a failing regression before repairing an actual bug.
Review-only workflows must not apply fixes without the configured approval.

Findings should state severity, confidence, file and line, a concrete failing
scenario, and a proposed repair. Separate observed failures from hypotheses.
