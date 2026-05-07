# 140 Control Gateway SPA

## Objective

Add an optional web gateway and TypeScript SPA for operator visibility, steering, human feedback queues, events, memory, and MCP-compatible control.

## Implementation

- Add `ui/control-plane-web` with a dependency-light TypeScript HTTP gateway and SPA.
- Proxy reads and workflow signals to Restate, goal-store, notifier, event-gateway, runner-registry, and memory-gateway.
- Add global goal and task list projections to `coat-goal-store` for dashboard views.
- Show agent progress, current projected prompts, task payloads, execution profiles, memory, events, artifacts, and human queues.
- Expose `/mcp` tools for overview, goal snapshot, agent activity, human threads, steering, memory search, and event-source listing.
- Add Compose and Kubernetes service definitions.

## Tests

- Compile the TypeScript gateway.
- Run `cargo test --workspace` and `cargo fmt --all --check`.
- Validate Compose with `docker compose -f infra/compose/docker-compose.yml config` when Docker is available.
- Validate Kubernetes with `kubectl apply --dry-run=client -f infra/k8s/base/all.yaml` when `kubectl` is available.
- Start the gateway locally and verify `/healthz` and `/mcp` tool listing.

## Acceptance

- The browser UI can inspect all projected goals and agent/task rows without becoming durable state.
- Operators can submit goals, steer, approve, cancel, inspect feedback threads, inspect events, and query/write memory through backend APIs.
- Agent/chat clients can use the MCP surface for the same read and steering functions.
- The Rust/Restate engine still runs without the gateway.
