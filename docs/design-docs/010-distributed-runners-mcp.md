# Design Doc: Distributed Runners, Model Routing, And MCP Context

## Intent

Durable tasks must be runnable by different worker processes on different nodes. Some workers may use Codex, some may use Claude Code, some may use hosted OpenAI or Bedrock models, and others may use local OpenAI-compatible providers such as vLLM, Ollama, or llama.cpp. The same routing layer is used for actor work, critic review, and review-unification tasks.

## Execution Profile

Every `TaskNode` has an `ExecutionProfile`:

- `RunnerSelector`: worker role, capabilities, labels, locality, and optional runner ID.
- `CapacityProvisioningPolicy`: registered-runner-only vs approved ephemeral capacity templates.
- `ModelRoute`: model candidates, provider kinds, routing strategy, required features, runtime parameters, and fallback policy.
- `PersonaSpec`: task-local persona and instruction references.
- `SubagentDelegationPolicy`: runner-context rule for durable child tasks vs native runner-local delegation.
- `McpContextRef`: MCP servers, allowed tools, secret refs, and propagation mode.
- `NotificationPolicy`: events and targets for feedback and approvals.
- `TaskPurpose`: actor work, candidate branch, branch vote, branch unification, critic review, review unification, actor retry, or research.

Child tasks inherit the parent execution profile unless they request an override.

Capacity provisioning is explicit. The default is `registered_runners_only`.
When a task can tolerate burst capacity, set `ExecutionProfile.capacity.mode` to
`prefer_registered_then_ephemeral` and reference one or more approved
`EphemeralRunnerTemplateRef` entries plus a `CapacityProvisionerPolicy`. The
coordinator or executor provisioner resolves those refs from configuration such
as the Helm `jattg-ephemeral-runner-templates` ConfigMap, creates a bounded Job
or temporary Restate service executor through a backend provisioner, waits for
registration, and then dispatches normally. In Kubernetes deployments the live
provisioner should use the Rust `kube`/`k8s-openapi` client path; rendered
manifests are fixtures and operator escape hatches. Workers do not create their
own Jobs from prompt text.

Dynamic capacity is a policy layer on top of this explicit provisioning model.
`ExecutionProfile.capacity.scaling` tells the coordinator and provisioner how to
turn durable demand into bounded supply. It is disabled by default. When enabled,
it uses:

- durable demand: runnable task frontier, unmatched tasks, running tasks,
  blocked tasks, event backlog, trigger pressure, and explicit priority boosts;
- current supply: runner-registry heartbeats, dispatchable runners,
  `capacity_remaining`, `max_concurrency`, stale/full flags, and pending
  provisions;
- policy limits: min/max runners, slots per runner, target backlog per runner,
  utilization target, headroom, cooldowns, max scale-up/down steps, and whether
  events can contribute to scaling.

The calculation answers "how many execution slots are needed for this pool?"
rather than "how many agents should an LLM spawn?" The coordinator groups demand
by role, pool, capability, labels, sandbox, model route, and locality. The
runner registry can produce a stateless recommendation through
`POST /capacity/plan`, but Restate/coordinator state remains authoritative.

Scale-up may create ephemeral runner or executor capacity only when all of these
are true:

- `CapacityScalingPolicy.enabled = true`;
- the mode is `provision_ephemeral`, not `manual` or `recommend_only`;
- `ExecutionProfile.capacity.mode` allows ephemeral provisioning;
- the template ref is approved and inside max runner/pending-provision limits;
- budget and approval policy allow the provision.

Scale-down is conservative. Persistent Deployments should use HPA/KEDA or
operator policy. Ephemeral runner Jobs should drain or expire through TTL,
`activeDeadlineSeconds`, and no-new-task assignment. The coordinator should not
kill active task runners just because the recommendation dropped.

## Subagent Delegation

COAT does not let worker processes create hidden subagent trees. The word
"subagent" means a coordinator-owned durable child task unless a future policy
explicitly states otherwise.

Default task behavior:

- `ExecutionProfile.subagents.mode = coordinator_durable_tasks`
- `ExecutionProfile.subagents.native_spawn = disabled`
- `ExecutionProfile.subagents.child_request_channel = agent_run_result_child_requests`

Runner initialization must inject this rule into Codex, Claude Code, OpenAI
Agents SDK, MCP client, and local-model contexts. If a worker decides more
agents are needed, it returns `ChildTaskRequest` values in
`AgentRunResult.child_requests`. The coordinator alone applies `SpawnPolicy`,
budget, approvals, memory context, MCP auth, sandbox profile, runner dispatch,
and model routing.

The bundled sidecars expose this through `/capabilities.subagents` and include
redacted diagnostics in every `/run-task` result. The Rust tool registry exposes
`subagent_policy` over MCP, and the control gateway exposes
`coat_subagent_policy`, so external chat/agent surfaces can initialize their
skill or system context with the same rule.

## Model Runtime Parameters

`ModelCandidate.params` is the typed place for model behavior that should be
visible to routing and review:

- `latency_class`: `fast`, `balanced`, `deep`, or `batch`;
- `speed_tier`: optional provider tier such as `speed`, `priority`, `flex`,
  `auto`, or `default` when the provider exposes speed or service-tier routing;
- `temperature_milli` and `top_p_milli`;
- `max_output_tokens`;
- `reasoning_effort`: `minimal`, `low`, `medium`, `high`, or `xhigh`;
- `timeout_seconds`;
- `extra` for provider-specific string values.

The generic model-provider runner reads these from `MODEL_PROVIDER_*` env vars
and exposes them through `/registration`, `/capabilities`, `/verify`, and
`AgentRunResult.model_used`. If a deployment uses a shared OpenAI-compatible
gateway such as Bifrost, LiteLLM, OpenRouter, or Docker Model Gateway, runners
can instead inherit `COAT_LLM_GATEWAY_URL`,
`COAT_LLM_GATEWAY_{WORK,RESEARCH,DEFAULT}_MODEL`, and
`COAT_LLM_GATEWAY_API_KEY`. That keeps provider keys centralized while still
letting the coordinator route durable work, research, review, and operator chat
model routes independently. `RUNNER_MODELS_JSON` can still override the whole candidate list
when a node serves multiple models. Labels remain for measured or operator
metadata such as `quality_tier`, `latency_ms`, GPU type, price, or pool. The
lowest-latency route uses measured latency labels first and falls back to typed
`latency_class` and optional `speed_tier`, so a fast local model can be
selected even before the runner has live latency observations.

The control gateway's `/api/chat` path is not a durable dispatch route and does
not use the runner fleet by default. Gateway chat is selected through
`COAT_CONTROL_CHAT_*`, `COAT_LLM_GATEWAY_*`, or direct OpenAI config, while
durable task runners are selected through `TaskNode.execution`, roles, personas,
labels, sandbox, and model route. If `COAT_CONTROL_CHAT_BACKEND=runner_registry`
is explicitly enabled, the gateway resolves only an operator-chat backend for a
user request and ignores unlabeled durable-work runners. A runner or model
candidate must opt in with labels such as `control_chat=true`,
`chat.intent=user_request`, or `routing_scope=operator_chat`; durable task
execution still goes through coordinator dispatch and runner task APIs.

## Runner Wrappers

COAT treats harnesses and providers as replaceable wrappers below the durable
task queue:

- `codex-runner-ts`: Codex App Server or Codex MCP boundary for bounded coding work.
- `claude-code-runner-ts`: generic Claude Code boundary for bounded tasks without the staff-engineer process bundle.
- `staff-engineer-runner-ts`: specialized `@ctxr/agent-staff-engineer` issue-to-PR lifecycle worker.
- `model-provider-runner-ts`: generic provider boundary for Bedrock, OpenAI-compatible APIs, vLLM, Ollama, llama.cpp, Hugging Face endpoints, and local processes.

Each wrapper exposes `/registration`, `/capabilities`, `/verify`, and
`/run-task`, supports stub mode for local smoke tests, self-registers when
`RUNNER_REGISTRY_URL` is set, and reports MCP/auth capabilities without leaking
secret values.

## Branch Competition

`GoalSpec.branching_policy` lets the coordinator create a `BranchGroup` for a root task, subgoal task, or explicit task ID. The target task can be cancelled, then multiple `candidate_branch` tasks run against the same prompt and dependencies. Each candidate may override role, execution profile, persona, model route, or prompt.

When candidates validate, the coordinator spawns `branch_vote` tasks using the configured voter roles. If `require_unification` is enabled, it then spawns one `branch_unification` task. The winning implementation is recorded through `BranchSelectionRequest` or by automatic policy such as `voter_quorum` or `highest_score`.

This is how COAT allows multiple subagents or models to solve the same goal without losing control of global state: candidates produce artifacts, voters produce structured `BranchVoteOutput`, and the coordinator records the selected task ID.

## Runner Registry

`coat-runner-registry` is the first control-plane service for distributed nodes.

Runners POST `RunnerRegistration` to `/runners`, send `/runners/heartbeat`, and the coordinator or operator can POST `/dispatch` with a task to receive a `RunnerDispatchDecision`. Operators can inspect `/runners/status` or `coat runner status` to see each runner's running tasks, remaining capacity, stale/full flags, and dispatchability. The bundled TypeScript sidecars self-register when `RUNNER_REGISTRY_URL` is configured and expose `/registration` plus `/capabilities` for inspection.

The registry is in-memory in this scaffold. It filters dispatch candidates by heartbeat lease and remaining capacity, but production should move runner state into Restate virtual objects or an indexed backing store.

The coordinator's Restate `AgentRunner` calls `/dispatch` as a journaled side effect, then invokes the matched runner's `/run-task` endpoint with `AgentRunRequest`. `AgentRunRequest.timeout_seconds` is derived from `GoalSpec.timeout_policy` and task budget, so slow or wedged runners produce a structured timeout result instead of blocking the control loop indefinitely. In local development, `COAT_ALLOW_LOCAL_STUB_FALLBACK=true` lets unmatched or unavailable runners fall back to a local stub. Production deployments should set this to `false` so unmatched tasks block and notify humans instead of pretending work ran.

`POST /capacity/plan` accepts `RunnerScalingRequest`. If the request omits
current supply, the registry derives pool supply from registered runner
heartbeats. Operator-facing CLI calls should fill an omitted/default policy from
`config.runner_capacity`, with pool-specific policy selected by pool key and the
default policy as fallback. The response is a `RunnerScalingDecision` with
per-pool desired runners, current runners, desired slots, provision count,
retirement suggestion, and reasons. This endpoint is advisory; the coordinator
or provisioner still owns approval, template resolution, cooldown enforcement,
and execution.

Dispatch decisions include:

- ranked eligible candidates with runner ID, node ID, endpoint, chosen model, score, and match reasons;
- rejected runners with explicit mismatch reasons;
- the MCP context ref that should be passed to the selected runner.

## Local Binary Tools

Some tasks need local binaries for validation or operator work: `git`, build
tools, package managers, `docker`, `helm`, `kubectl`, formal-methods binaries,
or project-specific CLIs. This is explicit task policy, not prompt-inferred
shell access.

`ExecutionProfile.local_tools` declares:

- enabled state and allowed `LocalToolPermission` entries;
- bare binary names, categories, risk, allowed subcommands, and denied args;
- whether the tool needs network, Docker socket, or cluster access;
- required runner capabilities and labels;
- timeout and bounded output limits;
- approval mode and command-evidence requirements.

Dispatch requires the selected runner to advertise `local_commands` plus
category-specific capabilities such as `git_cli`, `docker_cli`, `helm_cli`,
`kubernetes_cli`, `build_tooling`, or `package_manager_cli`. Tool-specific
labels such as `tools.helm=true` let operators distinguish installed binaries
from policy approval to use them.

The tool registry exposes local command access only as an MCP boundary:
`local_command` posts to the sandbox runner for `/commands/plan` or
`/commands/run`. The registry does not execute the command in-process.
`/commands/run` is still opt-in through `SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION`
and writes command evidence artifacts under the task workspace. When the MCP
request includes task-local `local_tools`, the sandbox runner enforces allowed
subcommands, denied arguments, denied binaries, policy timeouts, and output
limits before execution.

`RunnerLocality` can require any node, the coordinator node, a local-only runner, or a remote-only runner. The coordinator passes `COAT_COORDINATOR_NODE_ID` into dispatch when it is configured.

## Model Routing

Model routing is data, not hard-coded worker logic. A task can request:

- Codex;
- Claude Code;
- hosted OpenAI;
- Bedrock;
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

- access mode, defaulting to `single_user`;
- MCP server names and URIs;
- allowed tool lists;
- auth mode;
- `SecretRef` entries for env, Kubernetes Secret, Vault, cloud secret managers, 1Password, Bitwarden, Doppler, SOPS, local file, workload identity, external brokers, or OAuth delegation.
- `AuthDistributionPolicy`, which constrains whether credentials are runner-local, runner-resolved, coordinator-issued, workload-identity based, device-brokered, or externally brokered.

The runner resolves the secret reference at execution time using its node identity and local secret mounts. The coordinator can also issue short-lived context when `propagation = coordinator_issued`. Device/browser auth for Codex and Claude Code should usually use `propagation = runner_local_only` plus runner labels such as `auth.codex.device=true`; brokered user auth should use `oauth_device_broker` or `external_broker`.

Multi-user OIDC is not the default. A task that sets `access_mode = multi_user_oidc` must include `UserPrincipalRef` and `OidcDelegationPolicy`, and dispatch requires a runner with `oidc_user_delegation` plus labels such as `auth.oidc.user_delegation=true` and the tenant label. MCP servers authenticate as that user only through short-lived brokered OIDC access tokens or leases. See `docs/design-docs/130-multi-user-oidc-mcp.md`.

The bundled sidecars inspect MCP context during `/run-task` and report redacted secret availability diagnostics. They never include secret values in `AgentRunResult`.

When `COAT_MEMORY_GATEWAY_URL` is configured, the sidecars also call `/memory/context` before task execution. The returned context pack is represented as a `memory_context` artifact and summarized with hit counts, adapter report counts, and failed-adapter counts in diagnostics. A context fetch failure is non-terminal for the runner; coordinator policy and reviewers decide whether missing memory should block, trigger research, or run `memory_repair`.

Approval gating happens before dispatch. `GoalSpec.approval_policy` evaluates the task sandbox, runner selector, MCP tools, secret references, and brokered user-auth requirements; a required approval creates durable `ApprovalRequest` state and notifies the task's `NotificationPolicy`. Runners never self-approve their own requested capabilities.
High-risk local tools such as Docker socket access, Helm/Kubernetes cluster access, or policy-marked critical binaries also create approval reasons before dispatch when `require_for_local_tool_execution=true`.

The Rust tool registry exposes a minimal MCP HTTP endpoint at `/mcp`. If `MCP_TOOL_TOKEN` is set, requests must include `Authorization: Bearer ...`. Compose leaves this unset by default for local smoke work; Kubernetes wires it from `jattg-agent-secrets`.

Currently implemented MCP methods:

- `tools/list`
- `tools/call` with `repo_status`
- `tools/call` with `test_command`, which reports that execution must go through the sandbox runner
- `tools/call` with `local_command`, which plans or executes allowlisted local binaries through the sandbox runner
- `tools/call` with `artifact_manifest`
- `tools/call` with `subagent_policy`, which returns the durable child-task rule for MCP client initialization

## Notifications

Notification routing is task-local. Approval requests and feedback requests should create or continue a human-facing thread, while durable workflow state remains in Restate.

The notifier service is intentionally generic: it accepts `NotificationRequest`,
records local threads, and can deliver to dashboard queues, generic webhooks,
Slack incoming webhooks, email outbox, SQS, GitHub, Linear, Jira, and PagerDuty
adapters. Use stable infrastructure such as SQS when notifications need durable
fanout, replay, dead-letter queues, or downstream automations outside the COAT
process. The event gateway also treats SQS as an inbound source by polling
registered queues through `coat event poll-sqs`, normalizing the body through
the generic event contract, and routing through the same approval and trigger
path as webhooks. Credentials must come from normal AWS
environment/profile/workload identity resolution or secret middleware, not from
task state.
