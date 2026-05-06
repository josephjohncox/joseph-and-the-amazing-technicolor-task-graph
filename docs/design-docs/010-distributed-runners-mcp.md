# Design Doc: Distributed Runners, Model Routing, And MCP Context

## Intent

Durable tasks must be runnable by different worker processes on different nodes. Some workers may use Codex, some may use hosted OpenAI models, and others may use local OpenAI-compatible providers such as vLLM.

## Execution Profile

Every `TaskNode` has an `ExecutionProfile`:

- `RunnerSelector`: worker role, capabilities, labels, locality, and optional runner ID.
- `ModelRoute`: model candidates, provider kinds, routing strategy, required features, and fallback policy.
- `PersonaSpec`: task-local persona and instruction references.
- `McpContextRef`: MCP servers, allowed tools, secret refs, and propagation mode.
- `NotificationPolicy`: events and targets for feedback and approvals.

Child tasks inherit the parent execution profile unless they request an override.

## Runner Registry

`jattg-runner-registry` is the first control-plane service for distributed nodes.

Runners POST `RunnerRegistration` to `/runners`, send `/runners/heartbeat`, and the coordinator or operator can POST `/dispatch` with a task to receive a `RunnerDispatchDecision`.

The registry is in-memory in this scaffold. The production version should move runner state into Restate virtual objects or an indexed backing store.

## Model Routing

Model routing is data, not hard-coded worker logic. A task can request:

- Codex;
- hosted OpenAI;
- OpenAI-compatible endpoints;
- vLLM;
- Ollama;
- llama.cpp;
- Hugging Face;
- local processes.

The runner must only claim a task when it can satisfy the route and required features such as tool use, JSON schema output, streaming, long context, or local weights.

## MCP Context

MCP context never carries raw tokens.

Tasks carry:

- MCP server names and URIs;
- allowed tool lists;
- auth mode;
- `SecretRef` entries for env, Kubernetes Secret, Vault, cloud secret managers, local file, workload identity, or OAuth delegation.

The runner resolves the secret reference at execution time using its node identity and local secret mounts. The coordinator can also issue short-lived context when `propagation = coordinator_issued`.

## Notifications

Notification routing is task-local. Approval requests and feedback requests should create or continue a human-facing thread, while durable workflow state remains in Restate.

The notifier service is intentionally generic: it accepts `NotificationRequest`, logs when no target is configured, and can later add Slack, email, webhook, GitHub, Linear, Jira, and PagerDuty adapters.
