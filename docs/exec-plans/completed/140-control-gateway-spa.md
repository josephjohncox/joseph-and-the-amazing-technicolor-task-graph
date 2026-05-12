# 140 Control Gateway SPA

## Objective

Add an optional web gateway and TypeScript SPA for operator visibility, steering, human feedback queues, events, memory, and MCP-compatible control.

## Implementation

- Add `ui/control-plane-web` with a TypeScript HTTP gateway plus a Vite/React product SPA. Do not embed HTML pages in `server.ts`.
- Proxy reads and workflow signals to Restate, goal-store, notifier, event-gateway, runner-registry, and memory-gateway.
- Add global goal and task list projections to `coat-goal-store` for dashboard views.
- Show goal progress, task graph shape, memory, plans, runners, and human queues through product-facing cards and graph views; keep raw internals behind explicit diagnostics.
- Add a polished light/dark/system appearance switcher that themes the SPA shell, React Flow graph, dialogs, forms, cards, and status surfaces.
- Add a chat-assisted authoring tab that can draft goals, plans, steering directives, and state explanations from plain language while keeping mutations behind existing explicit controls.
- Expose `/mcp` tools for overview, goal snapshot, agent activity, human threads, chat assistance, steering, memory search, and event-source listing.
- Add Compose and Kubernetes service definitions.
- Add an optional Kubernetes OIDC front-door example using OAuth2 Proxy while keeping single-user mode as the default engine behavior.

## Tests

- Compile the TypeScript gateway and type-check/build the Vite React SPA.
- Run control gateway smoke tests for browser-facing Vite assets, chat stub responses, and MCP tool listing.
- Smoke tests must also prove behavior: chat authoring preserves the operator objective, emits acceptance evidence, keeps initial executable tasks coordinator-owned, and MCP follow-up drafting preserves source plan/path/index while producing a structured durable-plan prompt.
- Smoke tests cover degraded backend visibility, gateway-assigned goal workflow IDs, unsupported workflow-handler rejection, and research-output-to-steering conversion.
- Smoke tests cover backend-backed goal snapshots, checkpoint history, human queues, approval routing, plan compilation, memory search/context/edit/event proxying, and steering submissions through the gateway contract.
- Smoke tests render the memory replacement preview status and before/after diff table with realistic ready and blocked payloads through the existing Vite/React stack.
- GitHub CI runs the control gateway smoke after building the TypeScript gateway and SPA.
- Run `cargo test --workspace` and `cargo fmt --all --check`.
- Validate Compose with `coat deploy local config` when Docker is available.
- Validate Kubernetes with `coat deploy cluster apply --dry-run=client` when `kubectl` is available.
- Start the gateway locally and verify `/healthz` and `/mcp` tool listing.

## Follow-Ups

- Coordinate full Compose browser E2E and token-broker smoke timing through `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
- Extend browser-level smoke tests from gateway-backed contract checks to a full Compose harness once CI can run the stack.
- Add a token-broker-backed multi-user MCP smoke test once a broker implementation is selected.

## Acceptance

- The browser UI can inspect all projected goals and agent/task rows without becoming durable state.
- Operators can submit goals, steer, approve, cancel, inspect feedback threads, inspect events, and query/write memory through backend APIs.
- Operators can use chat to draft structured payloads without giving the chat assistant authority to mutate durable state.
- Agent/chat clients can use the MCP surface for the same read and steering functions.
- The Rust/Restate engine still runs without the gateway.
