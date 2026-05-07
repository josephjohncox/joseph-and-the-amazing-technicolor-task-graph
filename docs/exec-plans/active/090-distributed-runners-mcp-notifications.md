# 090 Distributed Runners MCP Notifications

## Objective

Make runner placement, model routing, MCP context, and human-feedback notifications first-class task contracts.

## Implementation

- Add `ExecutionProfile` to `GoalSpec`, `TaskNode`, and `ChildTaskRequest`.
- Add runner registration, heartbeat, dispatch, and capability matching contracts.
- Add model-route contracts for Codex, Claude Code, Bedrock, OpenAI, OpenAI-compatible, vLLM, Ollama, llama.cpp, Hugging Face, and local-process providers.
- Add task-local persona contracts.
- Add task-local `local_tools` contracts for allowlisted local binaries and runner capabilities.
- Add MCP server refs, secret refs, and context propagation policy.
- Add default `single_user` MCP access mode and opt-in `multi_user_oidc` delegation contracts.
- Add notification policy and delivery-report contracts.
- Add `coat-runner-registry` and `coat-notifier` service surfaces.
- Add notifier delivery adapters for dashboard queue, thread, webhook, Slack incoming webhook, email outbox, SQS, tracker webhook, and PagerDuty Events API targets.
- Add inbound SQS event-source polling through the event gateway so durable queues can fan events into goals without bypassing coordinator routing.
- Rank dispatch candidates by model-route strategy and return rejected runners with mismatch reasons.
- Persist runner registrations and heartbeats through `COAT_RUNNER_REGISTRY_JOURNAL_PATH` for local multi-node restarts while still honoring heartbeat TTL and capacity.
- Add sidecar `/capabilities` endpoints for model, MCP, capacity, and review-contract inspection.
- Add generic Claude Code and model-provider sidecars beside Codex and staff-engineer wrappers.
- Run a multi-agent default Compose pool with distinct runner IDs for coding, review/test, research, local-model, Claude Code, model-provider, and staff-engineer lanes.
- Add interactive local provider auth setup through `coat setup local-auth` and `scripts/coat-local-provider-setup.sh` so hosted keys, Bedrock routing, Ollama/vLLM endpoints, and Chat tab model settings are configured without printing secrets.
- Add interactive `coat setup chat-client` so Codex and Claude Code can register the remote control gateway as an HTTP MCP server and install the single-source `coat-control-plane` skill, while explicit flags keep automation non-interactive.
- Wire services into Compose, Kubernetes, CLI, examples, and schemas.

## Tests

- Child tasks inherit execution profile and switch persona/role correctly.
- Runner dispatch matches role, capabilities, labels, model route, and MCP capability.
- Runner dispatch rejects local-tool tasks unless the runner advertises `local_commands` and the required binary-specific capabilities and labels.
- Multi-user OIDC tasks require `oidc_user_delegation` runner capability, required tenant labels, and brokered-user approval.
- Dispatch explains locality and MCP mismatches and ranks multiple compatible model providers.
- HTTP-level registry tests cover registration, heartbeat, stale/full filtering, status inspection, and dispatch through the service routes.
- Local auth setup offers an interactive wizard, prints secret-safe checks, writes `infra/compose/local-providers.env`, and keeps stub modes as the default.
- Chat-client setup offers an interactive wizard, writes MCP config from command arguments or prompt choices, installs skill Markdown from `skills/coat-control-plane/SKILL.md`, and uses structured MUST-level instructions for steering.
- SQS notification targets serialize a stable queue envelope and use standard AWS SDK credential, region, and endpoint resolution.
- SQS event sources use the same SDK credential chain, normalize message bodies through `GenericEventSource`, and delete messages only after successful ingest when configured.
- Schema generation includes runner, model, MCP, execution, and notification contracts.
- Compose config validates.

## Follow-Ups

- Add a multi-process registry smoke test in Compose once Docker is available in CI.
- Add live notification and event-source smoke tests for Slack, SQS/LocalStack inbound/outbound, tracker, and PagerDuty targets once test credentials are approved.
- Add live provider verification profiles for Codex App Server, Claude Code, Bedrock, vLLM, Ollama, Hugging Face endpoints, and OpenAI-compatible gateways after the auth setup command is exercised on real nodes.

## Acceptance

- `cargo test --workspace` passes.
- `make schemas` writes new schemas.
- Operators can register vLLM, Claude Code, and Bedrock runners using `examples/runner-vllm.json`, `examples/runner-claude-code.json`, and `examples/runner-bedrock-provider.json`.
- Operators can register a local tool runner with `examples/runner-local-tools.json` and submit `examples/goal-local-tool-execution.json`.
- Notifier records human-feedback threads and queue entries, and can deliver dashboard, generic webhook, Slack incoming webhook, email outbox, SQS, tracker webhook, and PagerDuty targets.
- Event gateway can poll SQS queues as registered sources and route the normalized events through the same human review, trigger, and steering path as other event ingress.
