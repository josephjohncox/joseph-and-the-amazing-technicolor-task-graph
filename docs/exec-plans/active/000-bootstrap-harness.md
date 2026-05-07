# 000 Bootstrap Harness

## Objective

Create the repo harness that lets agents and engineers navigate the project without rediscovering architecture decisions.

## Implementation

- Keep `AGENTS.md` concise and directive.
- Keep `ARCHITECTURE.md` as the durable system map.
- Keep source references in `docs/references/source-links.md`.
- Keep implementation work split into numbered execution plans.
- Add doc-gardening checks in CI to catch stale slugs and missing source-of-truth docs.

## Acceptance

- Root docs explain mission, service boundaries, tests, and deployment entrypoints.
- Every future subsystem has a specific execution plan.
- `Agent.md` points to `AGENTS.md`.
