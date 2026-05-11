# 000 Bootstrap Harness

## Objective

Create the repo harness that lets agents and engineers navigate the project without rediscovering architecture decisions.

## Implementation

- Keep `AGENTS.md` concise and directive.
- Keep `ARCHITECTURE.md` as the durable system map.
- Keep source references in `docs/references/source-links.md`.
- Keep implementation work split into numbered execution plans.
- Add doc-gardening checks in CI to catch stale slugs and missing source-of-truth docs.
- Add doc-gardening checks for the canonical `coat` command hierarchy and installed-CLI examples.
- Add doc-gardening checks that every backticked path in the `AGENTS.md` Source Of Truth section exists.
- Enforce canonical command lines in both `crates/cli/src/main.rs` and `docs/operations/cli.md`, including the `tool` surface and setup/login/model-index subcommands.
- Add `docs/README.md` as the documentation map and reading order.
- Require service and sidecar entrypoints to include purpose and architecture-reference headers.
- Add doc comments for public cross-service domain contracts.

## Follow-Ups

- Keep `docs/exec-plans/active/` current as new work appears, and move finished plans to `docs/exec-plans/completed/` only after acceptance evidence is recorded.
- Extend doc-gardening checks when a new source-of-truth doc, service entrypoint, or public contract becomes mandatory.
- Keep stale command checks aligned with the canonical hierarchy whenever the CLI tree changes.

## Acceptance

- Root docs explain mission, service boundaries, tests, and deployment entrypoints.
- Code entrypoints explain their service boundary and link to relevant architecture docs.
- Every future subsystem has a specific execution plan.
- `Agent.md` points to `AGENTS.md`.
- `make docs-check` fails when `AGENTS.md` names a missing source-of-truth file or directory.
- Docs and code-facing operator messages do not reintroduce legacy top-level Compose, Kubernetes, human-feedback, development-runner, or old environment-prefix usage.
- `make docs-check` fails when the canonical CLI command map drifts from the operations guide.
