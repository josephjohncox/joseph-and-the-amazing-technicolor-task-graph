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
- Add typed `CapacityScalingPolicy`, `RunnerScalingRequest`, and `RunnerScalingDecision` contracts so the coordinator can derive runner demand from durable task/event queues and ask the registry/provisioner for bounded capacity recommendations.
- Add `config.runner_capacity` as the standard project/user/profile surface for capacity scaling defaults; `coat runner capacity-plan` should fill omitted/default request policy from resolved config before calling the advisory registry endpoint.
- Add runner-registry `POST /capacity/plan` as an advisory endpoint that combines supplied or heartbeat-derived pool supply with policy limits.
- Add `scripts/coat-runner-registry-smoke.sh` and `make runner-smoke` as a no-Docker, no-live-credentials multi-process smoke surface for registration, heartbeat, status, dispatch, and capacity planning.
- Add `scripts/coat-compose-runner-smoke.sh` and `make compose-runner-smoke` as an optional Docker Compose smoke for the default registry plus sidecar runner pool.
- Add `tool_registry_url` as a standard service endpoint and `coat tool web-search` as the operator route for `coat_web_search` against the MCP tool registry.
- Add sidecar `/capabilities` endpoints for model, MCP, capacity, and review-contract inspection.
- Add generic Claude Code and model-provider sidecars beside Codex and staff-engineer wrappers.
- Run a multi-agent default Compose pool with distinct runner IDs for coding, review/test, research, local-model, Claude Code, model-provider, and staff-engineer runners.
- Add interactive local provider auth setup through `coat setup local-auth` and `scripts/coat-local-provider-setup.sh` so hosted keys, Bedrock routing, Ollama/vLLM endpoints, fast/speed-tier/balanced/deep model params, and Chat tab model settings are configured without printing secrets.
- Extend local provider setup to cover memory-store adapters and embedding models, with hosted choices from models.dev and local choices from Ollama/OpenAI-compatible endpoint discovery.
- Add interactive `coat setup chat-client` so Codex and Claude Code can register the remote control gateway as an HTTP MCP server and install the single-source `coat-control-plane` skill, while explicit flags keep automation non-interactive.
- Wire services into Compose, Kubernetes, CLI, examples, and schemas.

## Tests

- Child tasks inherit execution profile and switch persona/role correctly.
- Runner dispatch matches role, capabilities, labels, model route, and MCP capability.
- Runner dispatch rejects local-tool tasks unless the runner advertises `local_commands` and the required binary-specific capabilities and labels.
- Multi-user OIDC tasks require `oidc_user_delegation` runner capability, required tenant labels, and brokered-user approval.
- Dispatch explains locality and MCP mismatches and ranks multiple compatible model providers.
- HTTP-level registry tests cover registration, heartbeat, stale/full filtering, status inspection, and dispatch through the service routes.
- Capacity-scaling tests cover disabled/manual no-op behavior, event-weighted demand, policy-bounded scale-up, and heartbeat-derived pool supply through `/capacity/plan`.
- Local `make runner-smoke` starts the registry on an ephemeral localhost port with a temporary journal, registers at least two runners, asserts stale/full filtering, fails on unexpected dispatch routing, and verifies `/capacity/plan`.
- Optional `make compose-runner-smoke` uses the existing Compose services, validates sidecar `/capabilities`, waits for runner auto-registration, verifies explicit dispatch to `codex-runner-ts`, verifies `/capacity/plan`, and skips clearly when Docker is unavailable.
- GitHub CI runs `make runner-smoke` on the normal build job and exposes `make compose-runner-smoke` through a Docker-capable manual workflow dispatch.
- Tool-registry CLI tests cover `coat tool` help, generic tool call parsing, `coat_web_search` parsing, and the example `WebSearchRequest` contract.
- Local auth setup offers an interactive wizard, prints secret-safe checks, writes `infra/compose/local-providers.env`, keeps non-interactive output stubbed by default, and flips selected interactive runners to live mode.
- Login and SSO setup run as COAT commands, not documentation-only provider commands, and can immediately preflight the resulting env file.
- Model routing tests cover indexed fast model choices and typed runtime params for latency class, temperature, top-p, max output tokens, reasoning effort, and timeout.
- Memory setup tests cover hosted embedding model indexing, local OpenAI-compatible embedding URL derivation, Qdrant/embedding preflight pairing, and gateway failures before network I/O when model or dimensions are missing.
- Chat-client setup offers an interactive wizard, writes MCP config from command arguments or prompt choices, installs skill Markdown from `skills/coat-control-plane/SKILL.md`, and uses structured MUST-level instructions for steering.
- SQS notification targets serialize a stable queue envelope and use standard AWS SDK credential, region, and endpoint resolution.
- SQS event sources use the same SDK credential chain, normalize message bodies through `GenericEventSource`, and delete messages only after successful ingest when configured.
- Schema generation includes runner, model, MCP, execution, and notification contracts.
- Compose config validates.

## Follow-Ups

- Coordinate remaining live runner and provider verification proof through `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
- Promote the manual `make compose-runner-smoke` workflow to required or scheduled CI once the runner has Docker build capacity and image caches.
- Add live notification and event-source smoke tests for Slack, tracker, PagerDuty, Google Calendar, Outlook, OpenTelemetry, and additional provider adapters once test credentials are approved; SQS/LocalStack inbound/outbound proof is closed by the active plan's `make eventops-sqs-smoke` evidence.
- Add live provider verification profiles for Codex App Server, Claude Code, Bedrock, vLLM, Ollama, Hugging Face endpoints, and OpenAI-compatible gateways after the auth setup command is exercised on real nodes.

## Acceptance

- `cargo test --workspace` passes.
- `make schemas` writes new schemas.
- Operators can register vLLM, Claude Code, and Bedrock runners using `examples/runner-vllm.json`, `examples/runner-claude-code.json`, and `examples/runner-bedrock-provider.json`.
- Operators can register a local tool runner with `examples/runner-local-tools.json` and submit `examples/goal-local-tool-execution.json`.
- Operators can run `make runner-smoke` locally or in CI without Docker or live model credentials.
- Operators can run `make compose-runner-smoke` for optional Docker-backed verification of default sidecar auto-registration, dispatch, and capacity planning.
- Operators can inspect tool-registry tools with `coat tool list` and route research through `coat tool web-search --file examples/web-search-request.json`.
- Notifier records human-feedback threads and queue entries, and can deliver dashboard, generic webhook, Slack incoming webhook, email outbox, SQS, tracker webhook, and PagerDuty targets.
- Event gateway can poll SQS queues as registered sources and route the normalized events through the same human review, trigger, and steering path as other event ingress.
