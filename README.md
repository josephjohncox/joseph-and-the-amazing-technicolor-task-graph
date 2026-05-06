# Joseph and the Amazing Technicolor Task Graph

![Joseph and the Amazing Technicolor Task Graph](./assets/coat-logo.png)

A durable task-tree control plane for long-running agentic engineering work.

`coat` is the short operational slug for commands, packages, environment variables, service images, and deployment names.

The core idea is simple: Restate owns durable time and replay, Rust owns policy and state, Codex owns bounded code execution, and specialized workers produce structured evidence for the coordinator to validate.

## Quick Start

```sh
cargo test --workspace
buf lint
cargo run -p coat-domain --bin generate-schemas -- schemas
cargo run -p coat-cli -- init
```

Run the local stack:

```sh
docker compose -f infra/compose/docker-compose.yml up --build
```

Start the optional Postgres/pgvector operational store profile when you want SQL-backed dashboard and audit development:

```sh
docker compose -f infra/compose/docker-compose.yml --profile db up postgres
```

Submit a goal through Restate ingress. In local development, unmatched tasks can fall back to the local stub runner:

```sh
cargo run -p coat-cli -- goal submit \
  --title "Smoke goal" \
  --objective "Prove the durable task tree can accept and validate a task"
```

For non-trivial work, author a full `GoalSpec` instead of relying on title/objective defaults:

```sh
cargo run -p coat-cli -- goal draft \
  --title "Strict review goal" \
  --objective "Implement a bounded change with typed review doctrine, sourced research, passing tests, regenerated schemas, and reviewer acceptance." \
  --strict-review \
  --human-steered \
  --out examples/drafts/strict-review-goal.json
cargo run -p coat-cli -- goal lint --file examples/goal-clean-plan.json --strict
cargo run -p coat-cli -- goal submit --file examples/goal-template-structured.json
cargo run -p coat-cli -- goal progress --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
cargo run -p coat-cli -- goal tasks \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611 \
  --file examples/task-query-subgoal.json
```

See `docs/operations/goal-authoring.md` for the intake, memory preflight, research preflight, compiler, and critic loop used to turn vague operator requests into structured goals.

Strict goals can opt in to a review-doctrine standard library for code quality, testing, formal-methods, DDD/functional-DDD, style, and simplicity checks:

```sh
cargo run -p coat-cli -- goal review-checks
cargo run -p coat-cli -- goal lint --file examples/goal-review-doctrine.json --strict
cargo run -p coat-cli -- goal steer-standard \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756800 \
  --check deep_research \
  --topic "state-of-the-art libraries and review doctrine"
```

The doctrine library is typed and extensible: use built-in presets, add custom objectives/evidence/gates/subagents, and apply overrides per goal. See `docs/design-docs/090-review-doctrine-stdlib.md`.

Goals also carry restart, timeout, and branch-competition policy. Operators can restart a blocked/timed-out goal without creating a new workflow, branch a goal or subgoal into multiple candidate implementations, and select the winning branch after reviewer/tester votes:

```sh
cargo run -p coat-cli -- goal submit --file examples/goal-branching-competition.json
cargo run -p coat-cli -- goal branch \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/branch-request-root.json
cargo run -p coat-cli -- goal select-branch \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/branch-selection.json
cargo run -p coat-cli -- goal restart \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/restart-request-task.json
```

## Services

- `coat-coordinator`: Restate workflow, distributed runner handoff, local stub fallback, validation handler.
- `coat-event-gateway`: webhook, calendar, scheduled-event, and triggered-goal ingress.
- `coat-goal-store`: queryable goal/task/event projection with local JSONL replay.
- `coat-runner-registry`: distributed runner registration, heartbeat, and task dispatch decisions.
- `coat-notifier`: notification and human-feedback delivery stub.
- `coat-memory-gateway`: local memory write/search/join/events gateway with MCP-shaped tools.
- `coat-validator`: standalone validation service.
- `coat-sandbox-runner`: workspace lifecycle and snapshot placeholder.
- `coat-tool-registry`: HTTP and MCP-shaped tool registry placeholder.
- `coat`: operator CLI, built from the `coat-cli` package.
- `codex-runner-ts`: Codex App Server or MCP worker boundary.
- `staff-engineer-runner-ts`: `@ctxr/agent-staff-engineer` worker boundary.
- `object-store`: local S3-compatible artifact store for large task outputs.

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
  docker compose -f infra/compose/docker-compose.yml --profile db up postgres goal-store
```

Inspect the projection surface:

```sh
cargo run -p coat-cli -- store policy
cargo run -p coat-cli -- store goal --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
cargo run -p coat-cli -- store tasks --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
cargo run -p coat-cli -- store events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
```

## Events And Schedules

External events enter through `coat-event-gateway` on `:9089`. Webhooks, CloudEvents-style payloads, calendar checks, queue messages, and cron jobs are normalized into `ExternalEvent`, deduped, and routed through `TriggeredGoalRequest`. They create or steer goals through Restate instead of invoking workers directly.

```sh
cargo run -p coat-cli -- event register --file examples/event-source-calendar-schedule.json
cargo run -p coat-cli -- event register --file examples/event-source-webhook-hmac.json
cargo run -p coat-cli -- event register \
  --file examples/event-source-webhook-hmac.json \
  --approval-id approval-123
cargo run -p coat-cli -- event ingest --file examples/external-event-calendar.json
cargo run -p coat-cli -- event trigger --file examples/triggered-goal-webhook.json
cargo run -p coat-cli -- event triggers
```

Webhook sources can require shared-secret headers, bearer tokens, or HMAC-SHA256 signatures with secrets resolved from `SecretRef`; production-only providers such as mTLS or OIDC JWT should be terminated by ingress or secret middleware until a provider adapter is installed. Use Kubernetes CronJobs for cluster scheduled triggers, provider push APIs or bounded pollers for calendars, and Restate timers for durable waits inside a running goal. Agent-proposed monitors or schedules should be reviewed and installed as event sources, not self-started by workers.

The public event contract is documented in `docs/api/event-gateway.asyncapi.yaml`. Kubernetes examples for a suspended scheduled trigger and optional pgvector-backed Postgres live under `infra/k8s/examples/`.

## Distributed Runners

Each durable task has an execution profile with runner selection, model candidates, persona, MCP context refs, timeout budget, result channels, and notification policy.

Example local vLLM runner registration:

```sh
cargo run -p coat-cli -- runner register --file examples/runner-vllm.json
cargo run -p coat-cli -- runner list
cargo run -p coat-cli -- runner status
cargo run -p coat-cli -- runner dispatch --file examples/dispatch-smoke.json
```

The bundled Codex and staff-engineer sidecars auto-register when `RUNNER_REGISTRY_URL` and `RUNNER_ENDPOINT` are set, which Compose and Kubernetes do by default.

Dispatch returns ranked candidates and rejected runners with reasons. Model routes can prefer first available, lowest latency, lowest cost, highest quality, weighted, or sticky-per-goal selection across Codex, hosted, and local OpenAI-compatible providers.

Branch competition uses the same routing layer: candidate tasks can use different personas, runner labels, or model routes, then branch-vote tasks and an optional unifier choose one implementation. The coordinator owns the branch group and selection record; workers only return structured evidence and votes.

Sidecars expose `/capabilities` for operator inspection and `/verify` for non-mutating dependency checks. The response includes roles, model candidates, MCP propagation support, active capacity, review-contract support, and live-mode readiness without exposing secret values.

When `COAT_MEMORY_GATEWAY_URL` is set, sidecars call `memory_context` before `/run-task` work and include a `memory_context` artifact plus redacted diagnostics in `AgentRunResult`. Context lookup failures do not fail the task; they are reported as diagnostics so a coordinator, reviewer, or operator can decide whether to continue, research, or repair memory adapters.

MCP auth is passed by reference, not by value. Runners resolve `SecretRef` entries from their local environment, Kubernetes, Vault, cloud secret stores, 1Password, Bitwarden, Doppler, SOPS material, workload identity, or an external broker. Device/browser logins such as Codex or Claude Code should normally be `runner_local_only` and constrained by runner labels; distributed user auth should use a brokered short-lived lease plus approval.

The tool registry exposes `/mcp` and requires `Authorization: Bearer ...` whenever `MCP_TOOL_TOKEN` is configured.

The notifier records local in-memory feedback threads. Operators can inspect them with:

```sh
cargo run -p coat-cli -- notify --threads
cargo run -p coat-cli -- notify --thread-key local-model-coding-smoke
```

## Result Channels

Workers report durable result locations through `AgentRunResult.git_result` and `AgentRunResult.object_artifacts`. Use git worktrees and task branches for source changes; use S3-compatible object storage for large generated outputs such as simulation runs, traces, datasets, screenshots, or reports. The coordinator stores refs, not large blobs or credentials.

Compose starts MinIO as `object-store` and initializes the `coat-artifacts` bucket. Kubernetes includes the same development object-store deployment; AWS/EKS should use real S3 by setting the `ObjectStoreRef` endpoint/region/bucket and resolving auth with workload identity or `SecretRef`.

## Review Gate

Goals use a bounded actor/critic review gate by default. Actor work must validate, critic review tasks fork from completed work, and a unifier joins the critic branches before `GoalState.satisfaction.satisfied` becomes true. If the reward score is too low, the coordinator can spawn a bounded actor retry and review it in a later round. Validation scores are kept as `LearningSignal` records for future routing and policy tuning.

Critics return structured `ReviewOutput` with a decision, reward, findings, and retry recommendation. A non-accept decision blocks satisfaction even if the numeric reward is high.

## Steering, Research, And Memory

Goals carry `control_policy`, `research_policy`, and `memory_policy`. Operators can steer a running goal by submitting `SteeringDirective` JSON:

```sh
cargo run -p coat-cli -- goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602 \
  --file examples/steering-request-research.json
```

Research tasks must return `ResearchOutput`: answer, sources, confidence, open questions, and an `InformationUsePlan` that tells the coordinator how to apply gathered information.

Clean goals carry `authoring` notes and a `plan` with stable subgoal IDs. `initial_tasks` are now materialized as child `TaskNode`s under the root planner task, so the coordinator can dispatch known work immediately while preserving the root as global planner. Use `coat goal progress` for a durable progress summary and `coat goal tasks` to find tasks by subgoal, role, status, purpose, tag, or runnable frontier.

Approval gates are task-local but governed by `GoalSpec.approval_policy`. Dangerous tasks move to `waiting_approval`, send an `approval_requested` notification, and resume when approved:

```sh
cargo run -p coat-cli -- approve \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602 \
  --approval-id <approval-request-id> \
  --approved true
```

The default memory substrate is hybrid: Zep/Graphiti exposed as an MCP memory server for temporal agent memory, plus Qdrant for embedded memory and RAG retrieval. Restate remains the durable workflow journal; Postgres/pgvector can be added as a queryable operational audit index when SQL joins and vector search should live together. `docs/design-docs/030-distributed-memory-knowledgebases.md` explains when to use FalkorDB, Neo4j, pgvector, Qdrant, LanceDB, or Tantivy.

Local memory gateway commands:

```sh
cargo run -p coat-cli -- memory write --file examples/memory-write-fact.json
cargo run -p coat-cli -- memory search --file examples/memory-search.json
cargo run -p coat-cli -- memory context --file examples/memory-context.json
cargo run -p coat-cli -- memory join --file examples/memory-join.json
cargo run -p coat-cli -- memory repair --file examples/memory-repair.json
cargo run -p coat-cli -- memory events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602
```

Create a sandbox workspace through the CLI:

```sh
cargo run -p coat-cli -- sandbox create --file examples/sandbox-workspace-request.json
```

Set `MEMORY_GATEWAY_JOURNAL_PATH` to make the local gateway replay an append-only JSONL journal on startup. Compose enables this with the `memory-gateway-data` volume.

Compose also runs Qdrant and configures the gateway to use it as the vector memory service. Set `OPENAI_API_KEY` or `MEMORY_GATEWAY_EMBEDDING_TOKEN` to enable the default OpenAI embedding path, or point `MEMORY_GATEWAY_EMBEDDING_URL` at an OpenAI-compatible local embedding server such as Hugging Face TEI. Set `MEMORY_GATEWAY_GRAPHITI_MCP_URL=http://localhost:8000/mcp/` to mirror gateway writes/searches/joins into Graphiti through best-effort MCP calls. Adapter success or failure is returned in `adapter_reports` without blocking local durability.

## Documentation

- Architecture: `ARCHITECTURE.md`
- Product spec: `docs/product-specs/coat-v1.md`
- Goal authoring: `docs/operations/goal-authoring.md`
- Memory/research design: `docs/design-docs/020-memory-research-steering.md`
- Distributed memory and knowledgebases: `docs/design-docs/030-distributed-memory-knowledgebases.md`
- Auth distribution: `docs/design-docs/040-auth-distribution.md`
- Execution plans: `docs/exec-plans/active/`
- Operations: `docs/operations/`
- Agent guide: `AGENTS.md`
