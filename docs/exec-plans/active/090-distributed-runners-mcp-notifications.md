# 090 Distributed Runners MCP Notifications

## Objective

Make runner placement, model routing, MCP context, and human-feedback notifications first-class task contracts.

## Implementation

- Add `ExecutionProfile` to `GoalSpec`, `TaskNode`, and `ChildTaskRequest`.
- Add runner registration, heartbeat, dispatch, and capability matching contracts.
- Add model-route contracts for Codex, Claude Code, Bedrock, OpenAI, OpenAI-compatible, vLLM, Ollama, llama.cpp, Hugging Face, and local-process providers.
- Add task-local persona contracts.
- Add MCP server refs, secret refs, and context propagation policy.
- Add default `single_user` MCP access mode and opt-in `multi_user_oidc` delegation contracts.
- Add notification policy and delivery-report contracts.
- Add `coat-runner-registry` and `coat-notifier` service surfaces.
- Rank dispatch candidates by model-route strategy and return rejected runners with mismatch reasons.
- Persist runner registrations and heartbeats through `COAT_RUNNER_REGISTRY_JOURNAL_PATH` for local multi-node restarts while still honoring heartbeat TTL and capacity.
- Add sidecar `/capabilities` endpoints for model, MCP, capacity, and review-contract inspection.
- Add generic Claude Code and model-provider sidecars beside Codex and staff-engineer wrappers.
- Wire services into Compose, Kubernetes, CLI, examples, and schemas.

## Tests

- Child tasks inherit execution profile and switch persona/role correctly.
- Runner dispatch matches role, capabilities, labels, model route, and MCP capability.
- Multi-user OIDC tasks require `oidc_user_delegation` runner capability, required tenant labels, and brokered-user approval.
- Dispatch explains locality and MCP mismatches and ranks multiple compatible model providers.
- Schema generation includes runner, model, MCP, execution, and notification contracts.
- Compose config validates.

## Follow-Ups

- Add multi-node integration tests that prove replayed stale heartbeats, locality labels, and capacity limits affect dispatch.
- Add notification adapters for Slack, email, webhook, and dashboard queues while keeping durable workflow state authoritative.

## Acceptance

- `cargo test --workspace` passes.
- `make schemas` writes new schemas.
- Operators can register vLLM, Claude Code, and Bedrock runners using `examples/runner-vllm.json`, `examples/runner-claude-code.json`, and `examples/runner-bedrock-provider.json`.
- Notifier records human-feedback threads and can deliver generic webhook targets with `SecretRef` bearer auth.
