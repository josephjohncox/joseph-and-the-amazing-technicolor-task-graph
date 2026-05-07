# 000 Bootstrap Harness

## Objective

Create the repo harness that lets agents and engineers navigate the project without rediscovering architecture decisions.

## Implementation

- Keep `AGENTS.md` concise and directive.
- Keep `ARCHITECTURE.md` as the durable system map.
- Keep source references in `docs/references/source-links.md`.
- Keep implementation work split into numbered execution plans.
- Add doc-gardening checks in CI to catch stale slugs and missing source-of-truth docs.
- Add `docs/README.md` as the documentation map and reading order.
- Require service and sidecar entrypoints to include purpose and architecture-reference headers.
- Add doc comments for public cross-service domain contracts.

## Acceptance

- Root docs explain mission, service boundaries, tests, and deployment entrypoints.
- Code entrypoints explain their service boundary and link to relevant architecture docs.
- Every future subsystem has a specific execution plan.
- `Agent.md` points to `AGENTS.md`.
