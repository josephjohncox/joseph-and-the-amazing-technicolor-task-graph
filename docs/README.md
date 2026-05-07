# COAT Documentation Map

This directory is the durable knowledge base for Joseph and the Amazing Technicolor Task Graph.

The short rule:

- product intent lives in `docs/product-specs/`;
- system architecture lives in `ARCHITECTURE.md` and `docs/design-docs/`;
- operator procedures live in `docs/operations/`;
- execution plans live in `docs/exec-plans/`;
- external source notes live in `docs/references/`;
- public API contracts live in `proto/`, `schemas/`, and `docs/api/`.

## Purpose

COAT builds a durable task-tree control plane for long-running agent work. The coordinator owns truth, Restate owns time, Rust owns policy and state, and runners do bounded work. Documentation should reinforce that architecture everywhere a contributor might start reading.

## Core Reading Order

1. `../AGENTS.md`: contributor and agent operating rules.
2. `../README.md`: local commands and project overview.
3. `../ARCHITECTURE.md`: durable coordinator, service boundaries, and authority model.
4. `product-specs/coat-v1.md`: product intent, non-goals, and success criteria.
5. `operations/local-dev.md`: local validation and smoke workflows.
6. `operations/runner-context-initialization.md`: runner, MCP, and skill context rules.
7. `operations/chat-client-integration.md`: Codex, Claude Code, skill, and remote MCP chat-client setup.
8. `operations/ephemeral-kubernetes-runners.md`: burst runners, temporary Restate executors, and toolbox injection.
9. `operations/operator-install.md`: provider-neutral production installation path.
10. `operations/releases.md`: version bumps, binary releases, and Helm chart releases.

## Design Docs

- `design-docs/000-system-shape.md`: smallest system map.
- `design-docs/010-distributed-runners-mcp.md`: runner placement, model routing, MCP context, and dispatch.
- `design-docs/020-memory-research-steering.md`: research and steering loop.
- `design-docs/030-distributed-memory-knowledgebases.md`: Graphiti/Zep, Qdrant, Postgres/pgvector, and memory policy.
- `design-docs/040-auth-distribution.md`: device auth, secret refs, workload identity, brokers, and OIDC delegation.
- `design-docs/050-goal-authoring-progress.md`: clean goal and progress contracts.
- `design-docs/060-result-channels-git-object-storage.md`: git and object-store result refs.
- `design-docs/070-protobuf-goal-store-protocols.md`: protobuf and read-model projection rules.
- `design-docs/080-events-webhooks-schedules.md`: event gateway, webhooks, calendars, and scheduled goals.
- `design-docs/090-review-doctrine-stdlib.md`: typed review objectives and validation gates.
- `design-docs/100-strong-sandboxing-guardrails.md`: gVisor, Kata, Firecracker, and executor guardrails.
- `design-docs/110-control-gateway-spa.md`: optional SPA and MCP dashboard.
- `design-docs/120-durable-planning-mode.md`: durable plan drafting and compilation.
- `design-docs/130-multi-user-oidc-mcp.md`: opt-in multi-user OIDC MCP delegation.

## Code Documentation Expectations

Each service crate and sidecar should start with a short purpose/architecture header that answers:

- What boundary does this process own?
- What boundary must it not own?
- Which design docs explain the behavior?

Public domain contracts in `crates/domain` should have comments when the type is exchanged across services, appears in JSON schemas, or is important to safety, routing, auth, validation, or goal satisfaction.

## Keeping Docs Current

Run:

```sh
sh scripts/coat-doc-gardener.sh
make schemas
buf lint
```

Update docs when behavior, service boundaries, public contracts, deployment knobs, or safety posture change.

Active execution plans must include a `## Follow-Ups` section. Treat that section as the durable continuation surface for later sessions: append unresolved work there, remove items only when the acceptance evidence is recorded, and move completed plans to `docs/exec-plans/completed/` only after follow-ups are either closed or intentionally transferred.

Use `coat follow-ups` to list all active plan continuation items, and `coat follow-ups --json` when another tool or dashboard should consume the queue.
