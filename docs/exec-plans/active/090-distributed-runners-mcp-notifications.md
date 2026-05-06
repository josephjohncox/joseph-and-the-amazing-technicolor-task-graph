# 090 Distributed Runners MCP Notifications

## Objective

Make runner placement, model routing, MCP context, and human-feedback notifications first-class task contracts.

## Implementation

- Add `ExecutionProfile` to `GoalSpec`, `TaskNode`, and `ChildTaskRequest`.
- Add runner registration, heartbeat, dispatch, and capability matching contracts.
- Add model-route contracts for Codex, OpenAI, OpenAI-compatible, vLLM, Ollama, llama.cpp, Hugging Face, and local-process providers.
- Add task-local persona contracts.
- Add MCP server refs, secret refs, and context propagation policy.
- Add notification policy and delivery-report contracts.
- Add `coat-runner-registry` and `coat-notifier` service stubs.
- Rank dispatch candidates by model-route strategy and return rejected runners with mismatch reasons.
- Add sidecar `/capabilities` endpoints for model, MCP, capacity, and review-contract inspection.
- Wire services into Compose, Kubernetes, CLI, examples, and schemas.

## Tests

- Child tasks inherit execution profile and switch persona/role correctly.
- Runner dispatch matches role, capabilities, labels, model route, and MCP capability.
- Dispatch explains locality and MCP mismatches and ranks multiple compatible model providers.
- Schema generation includes runner, model, MCP, execution, and notification contracts.
- Compose config validates.

## Acceptance

- `cargo test --workspace` passes.
- `cargo run -p coat-domain --bin generate-schemas -- schemas` writes new schemas.
- Operators can register a vLLM runner using `examples/runner-vllm.json`.
