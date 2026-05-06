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
- Add `jattg-runner-registry` and `jattg-notifier` service stubs.
- Wire services into Compose, Kubernetes, CLI, examples, and schemas.

## Tests

- Child tasks inherit execution profile and switch persona/role correctly.
- Runner dispatch matches role, capabilities, labels, model route, and MCP capability.
- Schema generation includes runner, model, MCP, execution, and notification contracts.
- Compose config validates.

## Acceptance

- `cargo test --workspace` passes.
- `cargo run -p jattg-domain --bin generate-schemas -- schemas` writes new schemas.
- Operators can register a vLLM runner using `examples/runner-vllm.json`.
