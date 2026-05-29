# Joseph and the Amazing Technicolor Task Graph

![Joseph and the Amazing Technicolor Task Graph](./assets/coat-logo.png)

A durable task-tree control plane for long-running agentic engineering work.

COAT — Coordinator Of Agentic Tasks — is the short name for the installed CLI
and `COAT_*` runtime environment variables. Deployable package surfaces use
`jattg`: Helm chart name, Kubernetes namespace/labels, GitHub release
artifacts, and published service images.

The core idea is simple: Restate owns durable time and replay, Rust owns policy and state, Codex owns bounded code execution, and specialized workers produce structured evidence for the coordinator to validate.

## Quick Start

The docs assume the `coat` CLI is installed and available on `PATH`.
For checkout-local development, direnv can put the built CLI on `PATH`:

```sh
direnv allow
make build
coat guide --print
```

The checkout-local direnv setup defaults to `target/debug/coat`, which matches
`cargo build`. Use `COAT_BUILD_PROFILE=release` in `.envrc.local` and
`make build COAT_BUILD_PROFILE=release` when you want `target/release/coat` to
be the command on `PATH`.

```sh
make ci
cargo test --workspace
buf lint
make schemas
coat init
coat plan follow-ups
```

Print root help, print the command map, or explicitly open the limited guide:

```sh
coat
coat --help
coat guide --print
coat guide
coat setup config --list-profiles
coat setup config --show
```

The bare `coat` command prints root help. Interactive dialogue is explicit and
kept to setup, auth, chat-client installation, human queues, approvals, and
similar operator flows where a picker is useful.

Run the local stub smoke stack:

```sh
coat deploy local preflight --allow-stub-runners
coat deploy local up --allow-stub-runners
```

The default Compose stack starts multiple stub runners: Codex coding, Codex
review/test, Claude Code, staff-engineer, generic model-provider, research, and
host-local model runners. They all register with `coat-runner-registry` so the
coordinator can route tasks by role, capability, label, model route, and typed
model params instead of assuming one local agent.

After the stack is up, seed real coordinator-created demo goals for local UI/TUI
navigation with:

```sh
make bootstrap-goals
```

That creates a completed executor lifecycle goal, a pending approval goal, and a
pending human-prompt thunk. Use fixture seeding only for read-model tests:
`make bootstrap-fixture-goals`.

Set up local provider credentials and model endpoints when you want live hosted or local models. The no-flag command starts an interactive wizard; explicit flags keep setup scriptable:

```sh
coat setup local-auth
coat setup login --codex --claude --preflight
coat setup sso --profile my-aws-sso-profile --write-env --bedrock-live --preflight
coat deploy local up --env-file infra/compose/local-providers.env
```

When `infra/compose/local-providers.env` already exists, `coat setup local-auth`
loads it first and uses those values as the interactive defaults for auth modes,
endpoints, model IDs, runtime params, memory stores, embeddings, and Chat tab
settings.

`coat setup local-auth` refreshes the models.dev catalog before showing hosted
model or embedding choices, unless a cached catalog is newer than 60 minutes or
`COAT_MODEL_INDEX` points at an explicit operator-managed catalog. The explicit
`coat setup model-index refresh` command remains available for cache warm-up and
CI images. Hosted model and embedding selectors read that catalog instead of
compiled-in model IDs. Local model and embedding selectors query the configured
OpenAI-compatible/Ollama endpoint for currently served models and fall back to a
custom model-id prompt when the endpoint is offline.
Inspect hosted embedding choices with `coat setup model-index show --provider
openai --embeddings`.

`coat setup login --claude` runs Claude Code's documented `claude auth login`
flow, not the interactive chat entrypoint. Add `--claude-sso`,
`--claude-console`, or `--claude-email you@example.com` when the runner node
needs those Claude Code auth options.

The interactive setup flow includes indexed fast, speed-tier, fast-completions,
balanced, deep-review, xhigh reasoning, deterministic JSON/tool-output, and
custom runtime parameter choices for local and hosted model routes. Codex setup
is separate from OpenAI hosted model-provider setup: selecting the OpenAI hosted
surface writes the generic model-provider runner config and can also write the
research runner config. It can also run selected device/browser login, AWS SSO,
Ollama pull, and preflight steps directly after writing the env file.

`coat init` writes `.coat/project.json`, a non-secret project config used by
CLI preflight checks and profile defaults for `cli`, `local`, `restate-cloud`,
and `eks`. `~/.coat/config.json` can provide machine-local overrides; use
`coat --config-profile ...` for one-off profile selection and `COAT_CONFIG`
only when a machine should use a non-default user config file.
Endpoint-bearing commands inherit the active profile unless a flag overrides it.
Most commands warn or fail, depending on command risk, when the checkout is not
initialized.
`coat deploy local up` runs the same preflight as `coat deploy local preflight`
and refuses to start an all-stub stack unless `--allow-stub-runners` is passed.
It also invokes Docker Compose with `up --build` and a checkout fingerprint
that invalidates local service-image cache when source files change, while
keeping cargo and npm dependency caches warm.
Before startup it validates the resolved Compose stack and removes orphan
containers from older local topologies.
The interactive auth wizard flips selected runner profiles from `stub` to `live`;
the non-interactive template stays stubbed until you edit it.
See `docs/operations/configuration.md` for config layering and secret rules.

Install COAT into a primary chat client with the remote HTTP MCP gateway and
the `coat-control-plane` skill. The no-flag command starts an interactive
wizard for gateway URL, token mode, Claude scope, and selected install actions:

```sh
export COAT_CONTROL_MCP_TOKEN=<redacted-token>
coat setup chat-client
coat setup chat-client \
  --mcp-url http://localhost:9090/mcp \
  --install-codex-mcp \
  --install-codex-skill
coat setup chat-client \
  --mcp-url http://localhost:9090/mcp \
  --install-claude-mcp \
  --install-claude-skill
```

The optional web control surface is included in Compose at `http://localhost:9090`. It shows goal progress, agent/task state, current projected prompts, runner status, execution-plan follow-ups, human feedback threads, event sources, schedules/triggers, and memory search/context. It also exposes a small MCP surface at `POST /mcp` for agent or chat clients.

Run the local services against Restate Cloud for personal durable use:

```sh
coat --config-profile restate-cloud deploy local up --restate-cloud --init-env
# edit infra/compose/restate-cloud.env with env id, API key, region, and signing public key
coat --config-profile restate-cloud deploy local config --restate-cloud
coat --config-profile restate-cloud deploy local up --restate-cloud --register-cloud --allow-stub-runners
```

`coat --config-profile restate-cloud deploy local up --restate-cloud` creates `infra/compose/restate-cloud.env`
from the example when it is missing and stops before starting containers if
placeholder cloud values are still present. `--register-cloud` starts Compose
detached and then runs `restate deployments register` for the default
`jattg-personal` tunnel. Pass `--tunnel-name` if you changed the tunnel name.

See `docs/operations/restate-cloud.md` for personal Restate Cloud, public endpoint, and Kubernetes operator paths. Kubernetes remains under `coat deploy cluster` and Helm chart commands rather than `coat deploy local`.

Render and validate the base Kubernetes manifest with the CLI:

```sh
coat deploy cluster render --output infra/k8s/rendered.yaml
coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client
coat deploy cluster executor-job render \
  --launch-plan examples/sandbox-launch-plan-kubernetes-job.json \
  --output /tmp/jattg-executor-job.json
```

Validate or install the packaged Helm chart with the CLI:

```sh
coat deploy chart lint
coat deploy chart template --output /tmp/jattg.yaml
coat deploy chart upgrade --values path/to/operator-values.yaml --wait
```

Start the optional Postgres/pgvector operational store profile when you want SQL-backed dashboard and audit development:

```sh
coat deploy local up --allow-stub-runners --profile db postgres
```

Submit a goal through Restate ingress. In local development, unmatched tasks can fall back to the local stub runner:

```sh
coat goal submit \
  --title "Smoke goal" \
  --objective "Prove the durable task tree can accept and validate a task"
export COAT_GOAL_ID=<goal-id-from-submit-output>
coat goal progress
```

For non-trivial work, author a full `GoalSpec` instead of relying on title/objective defaults:

```sh
coat plan draft \
  --title "Strict review goal plan" \
  --objective "Plan a bounded implementation with review doctrine before creating a GoalSpec." \
  --prompt "Capture questions, decisions, subgoals, and first tasks before agents execute."
coat plan list
coat plan compile \
  --plan-id <plan-id> \
  --strict-review \
  --human-steered \
  --out examples/drafts/strict-review-goal.json
coat goal draft \
  --title "Strict review goal" \
  --objective "Implement a bounded change with typed review doctrine, sourced research, passing tests, regenerated schemas, and reviewer acceptance." \
  --strict-review \
  --human-steered \
  --out examples/drafts/strict-review-goal.json
coat goal lint --file examples/goal-clean-plan.json --strict
coat goal submit --file examples/goal-template-structured.json
coat goal list
coat goal progress --latest
coat goal tasks \
  --latest \
  --file examples/task-query-subgoal.json
```

`GoalSpec.id` is optional in authored JSON. `coat goal submit` assigns one
when omitted and prints `goal_id`, `workflow_url`, and an `export COAT_GOAL_ID=...`
helper. Follow-up commands accept `--goal-id`, `COAT_GOAL_ID`, or `--latest`
through the goal-store projection.

See `docs/operations/goal-authoring.md` for the intake, memory preflight, research preflight, compiler, and critic loop used to turn vague operator requests into structured goals.
Use `docs/design-docs/120-durable-planning-mode.md` when the request needs a chat-style planning session before it becomes a durable goal.

Strict goals can opt in to a review-doctrine standard library for code quality, testing, formal-methods, DDD/functional-DDD, style, and simplicity checks:

```sh
coat goal review-checks
coat goal lint --file examples/goal-review-doctrine.json --strict
coat goal steer-standard \
  --latest \
  --check deep_research \
  --topic "state-of-the-art libraries and review doctrine"
```

The doctrine library is typed and extensible: use built-in presets, add custom objectives/evidence/gates/subagents, and apply overrides per goal. See `docs/design-docs/090-review-doctrine-stdlib.md`.

Goals also carry restart, timeout, and branch-competition policy. Operators can restart a blocked/timed-out goal without creating a new workflow, branch a goal or subgoal into multiple candidate implementations, and select the winning branch after reviewer/tester votes:

```sh
coat goal submit --file examples/goal-branching-competition.json
coat goal branch \
  --latest \
  --file examples/branch-request-root.json
coat goal select-branch \
  --latest \
  --file examples/branch-selection.json
coat goal restart \
  --latest \
  --file examples/restart-request-task.json
```

## Services

- `coat-coordinator`: Restate workflow, distributed runner handoff, local stub fallback, validation handler.
- `coat-event-gateway`: webhook, calendar, scheduled-event, and triggered-goal ingress.
- `coat-goal-store`: queryable goal/task/event projection with local JSONL replay.
- `coat-plan-store`: durable planning-mode records served by `coat-goal-store`.
- `coat-runner-registry`: distributed runner registration, heartbeat, and task dispatch decisions.
- `coat-notifier`: notification, local human-feedback threads, and webhook delivery adapter.
- `coat-memory-gateway`: local memory write/search/join/events gateway with MCP-shaped tools.
- `coat-control-web`: optional TypeScript control gateway, SPA, and MCP dashboard surface.
- `coat-validator`: standalone validation service.
- `coat-sandbox-runner`: workspace lifecycle, launch-plan, attestation, and content-addressed snapshot service.
- `coat-tool-registry`: HTTP and MCP-shaped tool registry with confined repo status, sandbox delegation, and artifact lookup.
- `coat`: operator CLI, built from the `coat-cli` package.
- `codex-runner-ts`: Codex App Server or MCP worker boundary.
- `claude-code-runner-ts`: generic Claude Code worker boundary for bounded tasks.
- `staff-engineer-runner-ts`: `@ctxr/agent-staff-engineer` worker boundary.
- `model-provider-runner-ts`: hosted/local model-provider boundary for Bedrock, OpenAI-compatible APIs, vLLM, Ollama, llama.cpp, Hugging Face, and local processes.
- `object-store`: local S3-compatible artifact store for large task outputs.
- `jattg-agent-toolbox`: tool-rich ephemeral Kubernetes Job image with Rust services, `coat`, runner sidecars, git, curl, jq, Python, Node, and controlled injection hooks.

## Releases

Release packaging and version bumps are documented in `docs/operations/releases.md`. Use `coat release plan --version ...` to preview binary and chart tags, `coat release bump --version ...` to update version files only, and `coat release cut --version ...` to bump, commit, and tag the release.

GitHub publishes binaries, GHCR service images, and Helm charts through release workflows:

- `.github/workflows/release-binaries.yml` on tags like `v0.2.0`;
- `.github/workflows/release-helm.yml` on tags like `chart-v0.2.0`.

## Follow-Ups

Active execution plans keep durable continuation items under `## Follow-Ups`. Use `coat plan follow-ups` for a human-readable queue, or `coat plan follow-ups --json` for dashboard/automation input.

## Protocols And Goal Store

Buf-managed protobuf contracts live under `proto/coat/v1`.

```sh
buf lint
```

The coordinator keeps Restate workflow state authoritative, then projects typed snapshots into the goal store through durable `ctx.run` steps. Local Compose uses `coat-goal-store` with a JSONL journal on `:9088` by default. The same service can run with `COAT_GOAL_STORE_BACKEND=postgres` and `COAT_GOAL_STORE_DATABASE_URL=postgres://...`; in that mode it writes the standard Postgres read model, stores full protocol records in JSONB, and keeps indexed columns for goal/task/event/operator queries.

Postgres migrations live under `infra/db/migrations/`. They cover goals, tasks, goal events, approvals, artifacts, event inbox/outbox records, and an optional `pgvector` memory index.

Run the Postgres projection locally with:

```sh
COAT_GOAL_STORE_BACKEND=postgres \
  coat deploy local up --allow-stub-runners --profile db postgres goal-store
```

Inspect the projection surface:

```sh
coat store policy
coat store goals
coat store plans
coat store approvals --limit 50
coat store goal --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
coat store tasks --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
coat store events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
coat store checkpoints --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
coat store goal-approvals --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
coat store record-artifacts --file examples/goal-store-record-artifacts.json
```

The web gateway reads the same projection surface. It uses global goal and task lists for dashboard views, and per-goal snapshots combine `GoalWorkflow/status`, `GoalWorkflow/progress`, tasks, events, artifacts, and `TaskRecord.payload_json.prompt` for agent prompt visibility.

Durable planning mode is stored in the same service:

```sh
coat plan draft --file examples/plan-draft-durable-mode.json
coat plan revise --plan-id <plan-id> --file examples/plan-revision-answer-questions.json
coat plan compile --plan-id <plan-id> --strict-review --human-steered
coat plan vote-candidate --plan-id <source-plan-id> --file examples/plan-candidate-vote.json
coat plan select-candidate --plan-id <source-plan-id> --file examples/plan-candidate-selection.json
```

Plans are versioned drafts. Compiling returns a `GoalSpec`; it does not submit the goal. Branch votes and selections are stored on the source plan so competing plan candidates can be reviewed before one compiled candidate is promoted.

Task graph colors are optional visual hints. Subgoals and child tasks can carry a `color` so the Technicolor Task Graph is easier to scan, and the SPA shows those colors in goal task tables, agent activity, and plan continuity views. Color must not drive runner routing, budgets, validation, approvals, or coordinator policy.

## Events And Schedules

External events enter through `coat-event-gateway` on `:9089`. Generic JSON events, webhooks, CloudEvents-style payloads, calendar checks, queue messages, and cron jobs are normalized into `ExternalEvent`, deduped, and routed through `TriggeredGoalRequest`. They create or steer goals through Restate instead of invoking workers directly.

```sh
coat event register --file examples/event-source-calendar-schedule.json
coat event register --file examples/event-source-webhook-hmac.json
coat event register --file examples/event-source-generic-ci.json
coat event register \
  --file examples/event-source-webhook-hmac.json \
  --approval-id approval-123
coat event ingest --file examples/external-event-calendar.json
coat event emit --source-id ci-events --file examples/generic-event-ci-failed.json
coat event trigger --file examples/triggered-goal-webhook.json
coat event triggers
```

Generic sources are the default adapter for CI, git, issue tracker, chat, monitoring, database-change, memory, runner, and agent-topology events before a provider-specific adapter exists. Webhook sources can require shared-secret headers, bearer tokens, or HMAC-SHA256 signatures with secrets resolved from `SecretRef`; production-only providers such as mTLS or OIDC JWT should be terminated by ingress or secret middleware until a provider adapter is installed. Use Kubernetes CronJobs for cluster scheduled triggers, provider push APIs or bounded pollers for calendars, and Restate timers for durable waits inside a running goal. Agent-proposed monitors or schedules should be reviewed and installed as event sources, not self-started by workers.

The public event contract is documented in `docs/api/event-gateway.asyncapi.yaml`. Kubernetes examples for a suspended scheduled trigger and optional pgvector-backed Postgres live under `infra/k8s/examples/`.

## Distributed Runners

Each durable task has an execution profile with runner selection, model candidates, persona, MCP context refs, timeout budget, result channels, and notification policy.

Subagents are durable COAT child tasks. Runner contexts, skills, MCP clients, Codex, Claude Code, and local model adapters should treat the word "subagent" as `AgentRunResult.child_requests`, not as native in-process agent spawning. The coordinator owns the durable queue, budget checks, approvals, routing, memory context, and sandbox policy for every requested child. See `docs/operations/runner-context-initialization.md`.

Example local vLLM runner registration:

```sh
coat runner register --file examples/runner-vllm.json
coat runner register --file examples/runner-claude-code.json
coat runner register --file examples/runner-bedrock-provider.json
coat runner list
coat runner status
coat runner dispatch --file examples/dispatch-smoke.json
coat tool list
coat tool web-search --file examples/web-search-request.json
```

The bundled Codex and staff-engineer sidecars auto-register when `RUNNER_REGISTRY_URL` and `RUNNER_ENDPOINT` are set, which Compose and Kubernetes do by default.
The runner registry can persist registrations and heartbeats through `COAT_RUNNER_REGISTRY_JOURNAL_PATH`, so local multi-node smoke runs can restart the registry without losing the visible runner set. Stale heartbeat TTL and capacity still determine dispatchability after replay.

Ephemeral Kubernetes runners and temporary Restate executors use approved
capacity templates, not ad hoc worker-owned loops. A task can set
`ExecutionProfile.capacity.mode=prefer_registered_then_ephemeral` and reference
Helm-provided templates such as `codex-burst` or `model-provider-burst`; the
coordinator or executor provisioner creates bounded Jobs through the Kubernetes
control plane, waits for normal runner/Restate registration, and dispatches
through the same durable path. The backend path uses the Rust `kube` and
`k8s-openapi` client stack; manual manifest examples remain fixtures and escape
hatches. See
`docs/operations/ephemeral-kubernetes-runners.md`.
The CLI can still render the reusable example set for inspection:

```sh
coat deploy cluster ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
```

Dispatch returns ranked candidates and rejected runners with reasons. Model routes can prefer first available, lowest latency, lowest cost, highest quality, weighted, or sticky-per-goal selection across Codex, hosted, and local OpenAI-compatible providers.

Branch competition uses the same routing layer: candidate tasks can use different personas, runner labels, or model routes, then branch-vote tasks and an optional unifier choose one implementation. The coordinator owns the branch group and selection record; workers only return structured evidence and votes.

Model and executor clusters are described in `docs/operations/model-runner-clusters.md`, including GB10/DGX Spark nodes, Mac mini runners, vLLM, Ollama, embedding services, and mixed sandbox fleets. Runners should register real capabilities and labels instead of relying on prompt convention.

Sidecars expose `/capabilities` for operator inspection and `/verify` for non-mutating dependency checks. The response includes roles, model candidates, MCP propagation support, subagent delegation policy, active capacity, review-contract support, and live-mode readiness without exposing secret values.

When `COAT_MEMORY_GATEWAY_URL` is set, sidecars call `memory_context` before `/run-task` work and include a `memory_context` artifact plus redacted diagnostics in `AgentRunResult`. Context lookup failures do not fail the task; they are reported as diagnostics so a coordinator, reviewer, or operator can decide whether to continue, research, or repair memory adapters.

MCP auth is passed by reference, not by value. Runners resolve `SecretRef` entries from their local environment, Kubernetes, Vault, cloud secret stores, 1Password, Bitwarden, Doppler, SOPS material, workload identity, or an external broker. Device/browser logins such as Codex or Claude Code should normally be `runner_local_only` and constrained by runner labels; distributed user auth should use a brokered short-lived lease plus approval.

The default MCP access mode is `single_user`. Multi-user OIDC is opt-in: set `McpContextRef.access_mode=multi_user_oidc`, include a `UserPrincipalRef`, configure `OidcDelegationPolicy`, and route only to runners advertising `oidc_user_delegation` plus tenant/user labels. MCP servers authenticate as the user through short-lived brokered OIDC access tokens or leases; raw user tokens never enter task state. See `docs/design-docs/130-multi-user-oidc-mcp.md` and `examples/mcp-context-multi-user-oidc.json`.

The tool registry exposes `/tools/list` and `/mcp`. Use `coat tool list`, `coat tool call --name subagent_policy --file examples/tool-subagent-policy-request.json`, and `coat tool web-search --file examples/web-search-request.json` for operator smoke calls without hand-written JSON-RPC. It requires `Authorization: Bearer ...` whenever `COAT_TOOL_REGISTRY_TOKEN` or the shared `MCP_TOOL_TOKEN` is configured. Its `subagent_policy` tool returns the same durable-child-task rule for MCP clients. The control gateway exposes `coat_subagent_policy` for chat and dashboard surfaces.

The notifier records local in-memory feedback threads. Operators can inspect them with:

```sh
coat human notify --threads
coat human notify --thread-key local-model-coding-smoke
coat human notify --queue
```

It also accepts dashboard queue targets, Slack incoming webhook targets, generic webhook targets with `SecretRef` bearer auth, and email outbox targets through the same `NotificationRequest` contract. Human approval and feedback state still resumes through coordinator workflows; notifier delivery reports are visibility evidence, not the durable source of truth.

IDE/LSP diagnostics, branch updates, PR activity, and PR test failures are normal event sources. Register the examples below to let editor extensions, local git watchers, repository webhooks, or CI systems signal the durable task graph without invoking workers directly:

```sh
coat event register --file examples/event-source-ide-lsp.json
coat event register --file examples/event-source-branch-activity.json
coat event register --file examples/event-source-pr-ci-failure.json
coat event emit --source-id ide-lsp-diagnostics --file examples/generic-event-ide-lsp-diagnostics.json
coat event emit --source-id branch-activity --file examples/generic-event-branch-updated.json
coat event emit --source-id pr-ci-failures --file examples/generic-event-pr-ci-failed.json
```

The gateway records IDE signals under `payload._coat_ide` and branch/PR/CI signals under `payload._coat_change_activity` for routing, dashboards, and memory correlation.

## Control Gateway And SPA

`ui/control-plane-web` is an optional TypeScript gateway and Vite/React browser UI. It composes existing backend APIs; it does not dispatch workers or mutate durable state directly.

Use it as the user-facing manager for durable plan drafting, goal progress, technicolor task graph viewing, shared memory search/write flows, runner status, human approvals, notification threads, and high-level steering. The richer diagnostics and raw contracts stay available through explicit inspect controls, MCP, CLI, and backend APIs. The SPA includes light, dark, and system appearance modes so operators can keep a readable dashboard during long monitoring sessions.

Set `COAT_CONTROL_GATEWAY_TOKEN` for `/api/*` bearer auth and `COAT_CONTROL_MCP_TOKEN` for `/mcp`. See `docs/design-docs/110-control-gateway-spa.md`.

Use `docs/operations/chat-client-integration.md` to install the MCP server and
`coat-control-plane` skill into Codex or Claude Code. The same setup command can
write Claude Code `.mcp.json`, install personal skill copies, and print or run
the MCP registration commands.

## Result Channels

Workers report durable result locations through `AgentRunResult.git_result`, `AgentRunResult.object_artifacts`, and `AgentRunResult.checkpoints`. Use git worktrees and task branches for source changes; use S3-compatible object storage for large generated outputs such as simulation runs, traces, datasets, screenshots, or reports; use checkpoints for reviewable task history such as branch milestones, commits, tags, workspace snapshots, object archives, or metadata markers. The coordinator stores refs, not large blobs or credentials.

The goal store exposes checkpoint history at `/goal-store/goals/{goal_id}/checkpoints`; the control gateway includes it in goal snapshots and exposes `coat_checkpoint_history` over MCP.

Compose starts MinIO as `object-store` and initializes the `jattg-artifacts` bucket. Kubernetes includes the same development object-store deployment; AWS/EKS should use real S3 by setting the `ObjectStoreRef` endpoint/region/bucket and resolving auth with workload identity or `SecretRef`.

## Review Gate

Goals use a bounded actor/critic review gate by default. Actor work must validate, critic review tasks fork from completed work, and a unifier joins the critic branches before `GoalState.satisfaction.satisfied` becomes true. If the reward score is too low, the coordinator can spawn a bounded actor retry and review it in a later round. Validation scores are kept as `LearningSignal` records for future routing and policy tuning.

Critics return structured `ReviewOutput` with a decision, reward, findings, and retry recommendation. A non-accept decision blocks satisfaction even if the numeric reward is high.

## Steering, Research, And Memory

Goals carry `control_policy`, `research_policy`, and `memory_policy`. Operators can steer a running goal by submitting `SteeringDirective` JSON:

```sh
coat goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602 \
  --file examples/steering-request-research.json
```

Research tasks must return `ResearchOutput`: answer, sources, confidence, open questions, and an `InformationUsePlan` that tells the coordinator how to apply gathered information.

Clean goals carry `authoring` notes and a `plan` with stable subgoal IDs. `initial_tasks` are now materialized as child `TaskNode`s under the root planner task, so the coordinator can dispatch known work immediately while preserving the root as global planner. Use `coat goal progress` for a durable progress summary and `coat goal tasks` to find tasks by subgoal, role, status, purpose, tag, or runnable frontier.

Approval gates are task-local but governed by `GoalSpec.approval_policy`. Dangerous tasks move to `waiting_approval`, send an `approval_requested` notification, and resume when approved:

```sh
coat human approve \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602 \
  --approval-id <approval-request-id> \
  --approved true
```

The default memory substrate is hybrid: Zep/Graphiti exposed as an MCP memory server for temporal agent memory, plus Qdrant for embedded memory and RAG retrieval. Restate remains the durable workflow journal; Postgres/pgvector can be added as a queryable operational audit index when SQL joins and vector search should live together. `docs/design-docs/030-distributed-memory-knowledgebases.md` explains when to use FalkorDB, Neo4j, pgvector, Qdrant, LanceDB, or Tantivy.

Local memory gateway commands:

```sh
coat memory write --file examples/memory-write-fact.json
coat memory search --file examples/memory-search.json
coat memory context --file examples/memory-context.json
coat memory join --file examples/memory-join.json
coat memory repair --file examples/memory-repair.json
coat memory events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602
```

Create a sandbox workspace through the CLI:

```sh
coat sandbox plan --file examples/sandbox-workspace-request-gvisor.json
coat sandbox create --file examples/sandbox-workspace-request.json
coat sandbox create --file examples/sandbox-workspace-request-gvisor.json
```

`SandboxProfile.isolation` can request local workspace, container, gVisor, Kata, Firecracker, Kubernetes Job, namespace-jail, or provider-backed execution. `sandbox plan` renders the launch contract that a real executor can consume; `sandbox create` stores it as `sandbox-launch-plan.json` beside the workspace manifest. Local Compose only attests metadata-only workspaces. Production runners should return enforced `SandboxAttestation` evidence and can require executor output/security guardrail reviews through `ExecutionProfile.guardrails`; see `docs/design-docs/100-strong-sandboxing-guardrails.md`.

For Kubernetes task execution, `coat-sandbox-runner` exposes `POST /kubernetes/executor-jobs/provision`. Plan-only mode returns the ConfigMap and Job objects; when `SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`, server dry-run and apply modes use the Kubernetes API instead of shelling out to `kubectl`.

Git result channels are metadata-only by default. To create real task worktrees, set `SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true`, set `SANDBOX_APPROVED_GIT_REPO_ROOTS` to comma-separated local repo roots, and include `live_git_worktree.enabled=true` plus an approval ID in the sandbox request. This keeps branch/worktree communication available while preventing workers from creating worktrees against arbitrary repos.

Sandbox workspaces include `checkpoints/checkpoint-manifest.json`, and launch plans expose `COAT_CHECKPOINT_MANIFEST` so executors can append git-style or snapshot-style history refs without relying on prompt convention.

Set `MEMORY_GATEWAY_JOURNAL_PATH` to make the local gateway replay an append-only JSONL journal on startup. Compose enables this with the `memory-gateway-data` volume.

Compose runs Qdrant, but the gateway only uses vector or graph memory stores after they are selected in config. Use `coat setup local-auth` and choose **Memory stores and embedding models** to enable Qdrant, Graphiti/Zep MCP, OpenAI hosted embeddings, Ollama, vLLM, llama.cpp, Hugging Face, or another OpenAI-compatible embedding endpoint. The wizard discovers hosted embedding choices from the models.dev cache and local choices from the live `/models` or Ollama tags endpoint, then writes `MEMORY_GATEWAY_*` settings into `infra/compose/local-providers.env`. Adapter success or failure is returned in `adapter_reports` without blocking local JSONL durability.

## Documentation

- Architecture: `ARCHITECTURE.md`
- Documentation map: `docs/README.md`
- Product spec: `docs/product-specs/coat-v1.md`
- Goal authoring: `docs/operations/goal-authoring.md`
- Memory/research design: `docs/design-docs/020-memory-research-steering.md`
- Distributed memory and knowledgebases: `docs/design-docs/030-distributed-memory-knowledgebases.md`
- Auth distribution: `docs/design-docs/040-auth-distribution.md`
- Strong sandboxing and guardrails: `docs/design-docs/100-strong-sandboxing-guardrails.md`
- Control gateway and SPA: `docs/design-docs/110-control-gateway-spa.md`
- Durable planning mode: `docs/design-docs/120-durable-planning-mode.md`
- Model and runner clusters: `docs/operations/model-runner-clusters.md`
- Execution plans: `docs/exec-plans/active/`
- Operations: `docs/operations/`
- Agent guide: `AGENTS.md`

## License

This repository is source-available under the Business Source License 1.1
(`BUSL-1.1`) with an additional use grant for non-competing use and a Change
License of Apache-2.0 on May 7, 2030. See `LICENSE`.

This is intentionally not an OSI open-source license before the Change Date:
blocking commercial forks or competing cloud products is incompatible with the
Open Source Definition.
