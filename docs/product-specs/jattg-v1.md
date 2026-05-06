# Product Spec: JATTG v1

## Problem

Long-running agent systems fail when the agent loop owns too much: global plan, tool side effects, retry policy, context, and completion judgment. JATTG makes the loop durable and explicit by modeling the work as a task tree.

## Goal

Provide a deployable control plane that can accept a goal, create durable tasks, run bounded workers, validate artifacts, request human approval, and resume safely after restarts.

## Non Goals

- No unbounded autonomous shell loop.
- No worker-owned global plan.
- No merge or tracker Done automation without a human gate.
- No live agent dependency required for local smoke tests.

## Users

- Operators submitting goals and approving risky actions.
- Engineers adding worker integrations.
- Agents reading `AGENTS.md` and execution plans before making changes.

## Success Criteria

- `cargo test --workspace` passes.
- Schemas generate into `schemas/`.
- Compose can render and start service containers.
- Kubernetes manifests render and can be dry-run validated.
- A stub goal can complete through the coordinator contract.
- Live Codex and staff-engineer integrations can be enabled behind environment gates.
- Distributed runner registrations can route tasks to separate nodes and local model providers.
- MCP tool context is passed with references to auth material rather than embedded tokens.
- Notification policies can keep separate human-feedback threads moving.
