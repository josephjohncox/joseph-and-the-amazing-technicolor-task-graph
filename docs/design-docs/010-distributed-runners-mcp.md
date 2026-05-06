# Design Doc: Distributed Runners, Model Routing, And MCP Context

## Intent

Durable tasks must be runnable by different worker processes on different nodes. Some workers may use Codex, some may use hosted OpenAI models, and others may use local OpenAI-compatible providers such as vLLM. The same routing layer is used for actor work, critic review, and review-unification tasks.

## Execution Profile

Every `TaskNode` has an `ExecutionProfile`:

- `RunnerSelector`: worker role, capabilities, labels, locality, and optional runner ID.
- `ModelRoute`: model candidates, provider kinds, routing strategy, required features, and fallback policy.
- `PersonaSpec`: task-local persona and instruction references.
- `McpContextRef`: MCP servers, allowed tools, secret refs, and propagation mode.
- `NotificationPolicy`: events and targets for feedback and approvals.
- `TaskPurpose`: actor work, candidate branch, branch vote, branch unification, critic review, review unification, actor retry, or research.

Child tasks inherit the parent execution profile unless they request an override.

## Branch Competition

`GoalSpec.branching_policy` lets the coordinator create a `BranchGroup` for a root task, subgoal task, or explicit task ID. The target task can be cancelled, then multiple `candidate_branch` tasks run against the same prompt and dependencies. Each candidate may override role, execution profile, persona, model route, or prompt.

When candidates validate, the coordinator spawns `branch_vote` tasks using the configured voter roles. If `require_unification` is enabled, it then spawns one `branch_unification` task. The winning implementation is recorded through `BranchSelectionRequest` or by automatic policy such as `voter_quorum` or `highest_score`.

This is how COAT allows multiple subagents or models to solve the same goal without losing control of global state: candidates produce artifacts, voters produce structured `BranchVoteOutput`, and the coordinator records the selected task ID.

## Runner Registry

`coat-runner-registry` is the first control-plane service for distributed nodes.

Runners POST `RunnerRegistration` to `/runners`, send `/runners/heartbeat`, and the coordinator or operator can POST `/dispatch` with a task to receive a `RunnerDispatchDecision`. Operators can inspect `/runners/status` or `coat runner status` to see each runner's running tasks, remaining capacity, stale/full flags, and dispatchability. The bundled TypeScript sidecars self-register when `RUNNER_REGISTRY_URL` is configured and expose `/registration` plus `/capabilities` for inspection.

The registry is in-memory in this scaffold. It filters dispatch candidates by heartbeat lease and remaining capacity, but production should move runner state into Restate virtual objects or an indexed backing store.

The coordinator's Restate `AgentRunner` calls `/dispatch` as a journaled side effect, then invokes the matched runner's `/run-task` endpoint with `AgentRunRequest`. `AgentRunRequest.timeout_seconds` is derived from `GoalSpec.timeout_policy` and task budget, so slow or wedged runners produce a structured timeout result instead of blocking the control loop indefinitely. In local development, `COAT_ALLOW_LOCAL_STUB_FALLBACK=true` lets unmatched or unavailable runners fall back to a local stub. Production deployments should set this to `false` so unmatched tasks block and notify humans instead of pretending work ran.

Dispatch decisions include:

- ranked eligible candidates with runner ID, node ID, endpoint, chosen model, score, and match reasons;
- rejected runners with explicit mismatch reasons;
- the MCP context ref that should be passed to the selected runner.

`RunnerLocality` can require any node, the coordinator node, a local-only runner, or a remote-only runner. The coordinator passes `COAT_COORDINATOR_NODE_ID` into dispatch when it is configured.

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

Implemented strategy behavior:

- `first_available`: lowest model priority wins.
- `highest_quality`: quality tier, feature coverage, context window, then priority.
- `lowest_latency`: uses model labels such as `latency_ms`.
- `lowest_cost`: uses model labels such as `cost_microusd_per_1k`.
- `weighted`: deterministic weighted selection by goal and task.
- `sticky_per_goal`: deterministic weighted selection by goal only.

## MCP Context

MCP context never carries raw tokens.

Tasks carry:

- MCP server names and URIs;
- allowed tool lists;
- auth mode;
- `SecretRef` entries for env, Kubernetes Secret, Vault, cloud secret managers, 1Password, Bitwarden, Doppler, SOPS, local file, workload identity, external brokers, or OAuth delegation.
- `AuthDistributionPolicy`, which constrains whether credentials are runner-local, runner-resolved, coordinator-issued, workload-identity based, device-brokered, or externally brokered.

The runner resolves the secret reference at execution time using its node identity and local secret mounts. The coordinator can also issue short-lived context when `propagation = coordinator_issued`. Device/browser auth for Codex and Claude Code should usually use `propagation = runner_local_only` plus runner labels such as `auth.codex.device=true`; brokered user auth should use `oauth_device_broker` or `external_broker`.

The bundled sidecars inspect MCP context during `/run-task` and report redacted secret availability diagnostics. They never include secret values in `AgentRunResult`.

When `COAT_MEMORY_GATEWAY_URL` is configured, the sidecars also call `/memory/context` before task execution. The returned context pack is represented as a `memory_context` artifact and summarized with hit counts, adapter report counts, and failed-adapter counts in diagnostics. A context fetch failure is non-terminal for the runner; coordinator policy and reviewers decide whether missing memory should block, trigger research, or run `memory_repair`.

Approval gating happens before dispatch. `GoalSpec.approval_policy` evaluates the task sandbox, runner selector, MCP tools, secret references, and brokered user-auth requirements; a required approval creates durable `ApprovalRequest` state and notifies the task's `NotificationPolicy`. Runners never self-approve their own requested capabilities.

The Rust tool registry exposes a minimal MCP HTTP endpoint at `/mcp`. If `MCP_TOOL_TOKEN` is set, requests must include `Authorization: Bearer ...`. Compose leaves this unset by default for local smoke work; Kubernetes wires it from `coat-agent-secrets`.

Currently implemented MCP methods:

- `tools/list`
- `tools/call` with `repo_status`
- `tools/call` with `test_command`, which reports that execution must go through the sandbox runner
- `tools/call` with `artifact_manifest`

## Notifications

Notification routing is task-local. Approval requests and feedback requests should create or continue a human-facing thread, while durable workflow state remains in Restate.

The notifier service is intentionally generic: it accepts `NotificationRequest`, logs when no target is configured, and can later add Slack, email, webhook, GitHub, Linear, Jira, and PagerDuty adapters.
