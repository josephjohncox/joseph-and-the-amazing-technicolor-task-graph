# Local Development

## Build

```sh
make ci
cargo check --workspace
cargo test --workspace
buf lint
make schemas
sh scripts/coat-doc-gardener.sh
npm run --prefix sidecars/codex-runner-ts build
npm run --prefix sidecars/staff-engineer-runner-ts build
npm run --prefix ui/control-plane-web build
```

## Compose

```sh
docker compose -f infra/compose/docker-compose.yml config
docker compose -f infra/compose/docker-compose.yml up --build
docker compose -f infra/compose/docker-compose.yml --profile db up postgres
docker compose \
  --env-file infra/compose/restate-cloud.env \
  -f infra/compose/docker-compose.yml \
  -f infra/compose/docker-compose.restate-cloud.yml \
  --profile restate-cloud \
  config
docker compose \
  --env-file infra/compose/restate-cloud.env.example \
  -f infra/compose/docker-compose.yml \
  -f infra/compose/docker-compose.restate-cloud.yml \
  --profile restate-cloud \
  config
```

Restate ingress is exposed on `http://localhost:8080`.
When using the Restate Cloud profile, cloud ingress is exposed through the tunnel on `http://localhost:18080` by default.
The coordinator service listens internally on `http://coordinator:9080`.
The control gateway and SPA listen on `http://localhost:9090`.
The Codex and staff-engineer sidecars auto-register with `runner-registry` when Compose starts.
The sandbox runner uses `SANDBOX_WORKSPACE_ROOT=/workspaces` in Compose and writes per-task manifests under the `sandbox-workspaces` volume.
Live git worktree creation is disabled by default. For an explicitly approved local development run, start the sandbox runner with:

```sh
SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true
SANDBOX_APPROVED_GIT_REPO_ROOTS=/absolute/path/to/approved/repo/root
SANDBOX_REQUIRE_LIVE_GIT_WORKTREE_APPROVAL=true
```

Then include `live_git_worktree.enabled=true` and `live_git_worktree.approval_id` in the sandbox create request. Without all three controls, the runner still returns a planned `git_result` but does not mutate the repo.

## CLI

```sh
coat init
coat plan draft --file examples/plan-draft-durable-mode.json
coat plan list
coat plan revise \
  --plan-id <plan-id> \
  --file examples/plan-revision-answer-questions.json
coat plan compile \
  --plan-id <plan-id> \
  --strict-review \
  --human-steered \
  --out examples/drafts/compiled-goal.json
coat compose up --restate-cloud
coat goal draft \
  --title "Local strict review smoke" \
  --objective "Create a strict-review goal draft with typed doctrine and a bounded initial frontier." \
  --strict-review \
  --human-steered \
  --out examples/drafts/local-strict-review.json
coat goal review-checks
coat goal submit --title "Smoke" --objective "Run a registered stub sidecar task"
coat goal submit --file examples/goal-template-structured.json
coat runner register --file examples/runner-vllm.json
coat runner list
coat runner status
coat runner dispatch --file examples/dispatch-smoke.json
coat event register --file examples/event-source-calendar-schedule.json
coat event register --file examples/event-source-webhook-hmac.json
coat event register --file examples/event-source-generic-ci.json
coat event register --file examples/event-source-slack-event.json
coat event register --file examples/event-source-stripe-webhook.json
coat event register \
  --file examples/event-source-webhook-hmac.json \
  --approval-id approval-123
coat event ingest --file examples/external-event-calendar.json
coat event emit \
  --source-id ci-events \
  --file examples/generic-event-ci-failed.json
coat event trigger --file examples/triggered-goal-webhook.json
coat event triggers
coat goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602 \
  --file examples/steering-request-research.json
coat goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756800 \
  --file examples/steering-standard-abstraction.json
coat goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756800 \
  --file examples/steering-standard-deep-research.json
coat goal steer-standard \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756800 \
  --check abstraction \
  --topic "durable coordinator task graph" \
  --emit-only
coat goal branch \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/branch-request-root.json
coat goal select-branch \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/branch-selection.json
coat goal restart \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/restart-request-task.json
coat notify --file examples/notification-approval.json
coat notify --file examples/notification-webhook.json
coat notify --threads
coat notify --thread-key local-model-coding-smoke
coat store policy
coat store goals
coat store plans
coat store all-tasks
coat store approvals --limit 50
coat store record-artifacts --file examples/goal-store-record-artifacts.json
coat sandbox create --file examples/sandbox-workspace-request.json
coat sandbox create --file examples/sandbox-workspace-request-live-git.json
coat memory write --file examples/memory-write-fact.json
coat memory search --file examples/memory-search.json
coat memory join --file examples/memory-join.json
coat memory events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602
coat restate cloud-env
coat restate register-cloud --dry-run
coat k8s render --output infra/k8s/rendered.yaml
```

Manual runner registration is only needed for external workers such as a vLLM node. Sidecars use:

Use `docs/operations/goal-authoring.md` before submitting non-trivial goals. It provides the intake, memory preflight, research preflight, compiler, and critic loop for producing structured `GoalSpec` JSON.

- `RUNNER_REGISTRY_URL`
- `RUNNER_ENDPOINT`
- `RUNNER_ID`
- `NODE_ID`
- `RUNNER_LABELS_JSON`
- `RUNNER_MODELS_JSON`
- `RUNNER_MCP_SERVERS_JSON`
- `OPENAI_API_KEY` for Codex API-key mode
- `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, or `CLAUDE_CODE_OAUTH_TOKEN` for non-interactive Claude Code modes

Sidecars also expose `GET /capabilities`, which is the quickest way to verify model candidates, review support, MCP propagation support, and remaining capacity without reading environment variables.

Use sidecar verification endpoints for non-mutating dependency checks:

```sh
curl -sS http://localhost:9091/verify
curl -sS http://localhost:9092/verify
```

Codex MCP and App Server probes are opt-in through `CODEX_VERIFY_MCP=1` and `CODEX_VERIFY_APP_SERVER=1` so a basic health check never starts live agent infrastructure by accident.

Local MCP smoke call:

```sh
curl -sS -X POST http://localhost:9084/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

Set `MCP_TOOL_TOKEN` to require bearer auth on `/mcp`.
In Compose, `tool-registry` mounts the sandbox workspace volume read-only and sets `TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT=/workspaces`, so `artifact_manifest` can return `workspace-manifest.json`, `sandbox-launch-plan.json`, `snapshots/latest.json`, and worker `artifacts/artifact-manifest.json` for a `{goal_id, task_id}` lookup. Kubernetes should normally resolve large artifacts from object storage or the goal-store projection; set the same env var only when the registry can safely read the sandbox workspace volume.
Compose also sets `TOOL_REGISTRY_SANDBOX_RUNNER_URL=http://sandbox-runner:9083`. With that URL configured, the MCP `test_command` tool posts to `coat-sandbox-runner /commands/plan` and returns an approval-aware command plan instead of executing commands in the tool registry process.

For local Codex or Claude Code device/browser auth, prefer a runner-local setup:

- log in on the node that runs the sidecar;
- label that runner with values such as `auth.codex.device=true` or `auth.claude.device=true`;
- set task `auth_distribution.mode` to `runner_local_only`;
- keep `allow_secret_sync=false`.

Use `examples/auth-distribution-codex-device.json` for the node-local shape and `examples/auth-distribution-claude-brokered.json` for a brokered human-auth shape. Brokered user auth should emit an approval or feedback notification with the device-code URL or browser-login instructions, then resume through durable approval state.

Default local mode is single-user. Multi-user OIDC MCP delegation is opt-in and should stay off for local smoke runs unless you are testing an auth broker. Use `examples/mcp-context-multi-user-oidc.json`, set runner capabilities to include `oidc_user_delegation`, label the runner with `auth.oidc.user_delegation=true` and tenant labels, and keep all user tokens in the broker or MCP server. Task state should contain only `UserPrincipalRef`, `OidcDelegationPolicy`, consent refs, and `SecretRef` values.

The notifier stores local in-memory notification threads by `feedback_thread_key`, thread target address, or goal ID. This is for operator visibility in local runs; Restate workflow state remains the source of truth for approval and feedback signals.
It also supports `NotificationTargetKind::webhook` by posting the `NotificationRequest` JSON to the target address and resolving optional bearer tokens through `SecretRef`. Slack, email, tracker, and paging targets intentionally require provider-specific adapters instead of pretending a local thread equals real delivery.

`coat-event-gateway` listens on `http://localhost:9089` in Compose. It records webhook, generic, calendar, scheduled, and bus events with dedupe keys, then can create or hold triggered goals. Set `COAT_EVENT_GATEWAY_TOKEN` to require bearer auth for mutating endpoints. Use `COAT_RESTATE_INGRESS` when the gateway should submit generated goals directly to Restate.

Local Compose defaults the event gateway to `COAT_EVENT_GATEWAY_BACKEND=jsonl`. To exercise the SQL event inbox/outbox path, start the database profile and set `COAT_EVENT_GATEWAY_BACKEND=postgres`; the service uses `COAT_EVENT_GATEWAY_DATABASE_URL`, falling back to the same Postgres database used by the goal store.

Generic sources use `GenericEventSource` to normalize arbitrary JSON or CloudEvents-compatible payloads. Register `examples/event-source-generic-ci.json`, then emit `examples/generic-event-ci-failed.json` with `coat event emit --source-id ci-events --file ...`. This is the default adapter for CI, git, issue tracker, chat, monitoring, database-change, memory, runner, and agent-topology events before a provider-specific adapter exists.

Webhook source auth is separate from gateway admin auth. Register a `WebhookAuthPolicy` with `kind=shared_secret_header`, `bearer_token`, or `hmac_sha256`, point `secret_ref` at an environment variable or local secret file for local smoke tests, and keep production secret providers behind Kubernetes, Vault, cloud secret stores, or workload identity. The example `examples/event-source-webhook-hmac.json` expects `COAT_GITHUB_WEBHOOK_SECRET`.
Provider presets cover GitHub HMAC, Slack Events API HMAC, and Stripe-style HMAC canonicalization. The Slack example expects `COAT_SLACK_SIGNING_SECRET`; the Stripe example expects `COAT_STRIPE_WEBHOOK_SECRET`.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` to smoke-test production activation policy. Risky enabled sources then need `coat event register --approval-id ...` on registration, or they should be registered with `"enabled": false` until a human approves activation.

The event API contract lives at `docs/api/event-gateway.asyncapi.yaml`. The cluster CronJob pattern lives at `infra/k8s/examples/calendar-trigger-cronjob.yaml`.

`coat-goal-store` listens on `http://localhost:9088` in Compose. It stores queryable goal, task, event, approval, and artifact projections in an append-only JSONL journal at `/data/goal-store.jsonl` by default. Restate remains authoritative; the goal store is for local operator inspection and future dashboards.

Inspect projections with:

```sh
coat store policy
coat store goal --goal-id <goal-id>
coat store tasks --goal-id <goal-id>
coat store events --goal-id <goal-id>
coat store artifacts --goal-id <goal-id>
coat store goal-approvals --goal-id <goal-id>
coat store record-artifacts --file examples/goal-store-record-artifacts.json
```

The web gateway uses the goal-store list endpoints for dashboard views:

```sh
curl -sS http://localhost:9090/api/plans
curl -sS http://localhost:9090/api/goals
curl -sS http://localhost:9090/api/agents
curl -sS http://localhost:9090/api/approvals
```

Per-goal views combine Restate workflow handlers with goal-store projection data so operators can inspect current task prompts from `TaskRecord.payload_json.prompt`:

```sh
curl -sS http://localhost:9090/api/goals/<goal-id>
```

The MCP dashboard surface is available at `POST /mcp`:

```sh
curl -sS -X POST http://localhost:9090/mcp \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

Set `COAT_CONTROL_GATEWAY_TOKEN` and `COAT_CONTROL_MCP_TOKEN` when exposing the gateway beyond local trusted development.

The service also has a real Postgres backend. Start the local database and goal store with:

```sh
COAT_GOAL_STORE_BACKEND=postgres \
  docker compose -f infra/compose/docker-compose.yml --profile db up postgres goal-store
```

Keep full contract payloads in JSONB and use typed columns for IDs, statuses, roles, subgoals, event kinds, and artifact URIs. Migration scaffolding lives under `infra/db/migrations/`; Compose starts a local `pgvector/pgvector:pg16` database with the `db` profile.

Approval requests appear in goal state under `approvals` and in notifier threads when the task notification policy includes `approval_requested`. Approve or reject with:

```sh
coat approve \
  --goal-id <goal-id> \
  --approval-id <approval-request-id> \
  --approved true
```

The default `ApprovalGatePolicy` requires approval for open network, non-isolated runners, secret-bearing MCP contexts, brokered user auth, dangerous MCP tools, privileged runner labels/capabilities, and any approval-policy `never` task that is not isolated.

Review output examples live under `examples/`. Runner implementations should return the same shape as `examples/review-output-changes-requested.json` when they need to block satisfaction and request an actor retry. Findings should include file, line, and priority when the issue is tied to code; high/critical findings and priority 0/1 findings block validation even if the reviewer accidentally returns `accept`.

Tester and code-worker results should include `test_evidence` entries with command, exit code, pass/fail, duration, and stdout/stderr or artifact URIs whenever `done_criteria.tests_pass=true`. The validator treats missing or all-failing test evidence as incomplete for work-like and tester tasks.

Research output examples live under `examples/research-output-memory-substrate.json`. A live research runner should replace stub sources with primary, official, or peer-reviewed sources and include a concrete information-use plan.

Graphiti/Zep memory is configured as a policy and MCP endpoint in this scaffold. For local experiments, run the upstream Graphiti MCP server and point task MCP context at `http://localhost:8000/mcp/`; keep credentials in `SecretRef` or environment, not in goal JSON.

`coat-memory-gateway` listens on `http://localhost:9087` in Compose. Set `MEMORY_GATEWAY_TOKEN` to require bearer auth and `COAT_MEMORY_GATEWAY_TOKEN` for CLI calls.

Set `MEMORY_GATEWAY_JOURNAL_PATH` to enable the local append-only JSONL journal. Compose sets it to `/data/memory-gateway.jsonl` and mounts `memory-gateway-data`, so local memory records replay after service restart.

Set `MEMORY_GATEWAY_GRAPHITI_MCP_URL=http://localhost:8000/mcp/` when a Graphiti MCP server is running and the gateway should mirror local memory operations into the graph. `MEMORY_GATEWAY_GRAPHITI_GROUP_ID` defaults to `coat`; per-goal `MemoryStoreRef.namespace` overrides it. Use `MEMORY_GATEWAY_GRAPHITI_TOKEN` only when the remote MCP endpoint requires bearer auth.

Compose runs Qdrant on `http://localhost:6333` and configures `MEMORY_GATEWAY_QDRANT_URL=http://qdrant:6333`. The gateway mirrors memory writes and joins into Qdrant when an embedding endpoint is available, then merges vector hits into `memory_search`.

Build a bounded worker context pack with:

```sh
coat memory context --file examples/memory-context.json
```

The response includes ranked memory hits, adapter reports, and an `InformationUsePlan` that tells the worker which facts are usable, which assumptions to avoid, and which validation checks to carry forward.

Default hosted embeddings:

- `MEMORY_GATEWAY_EMBEDDING_URL=https://api.openai.com/v1/embeddings`
- `MEMORY_GATEWAY_EMBEDDING_MODEL=text-embedding-3-large`
- `OPENAI_API_KEY` or `MEMORY_GATEWAY_EMBEDDING_TOKEN`

Local embeddings:

- Run Hugging Face Text Embeddings Inference or another OpenAI-compatible embedding server.
- Set `MEMORY_GATEWAY_EMBEDDING_URL=http://host.docker.internal:8080/v1/embeddings` for a host-local TEI server, or `http://tei:80/v1/embeddings` if you add TEI to the Compose network.
- Set `MEMORY_GATEWAY_EMBEDDING_MODEL=text-embeddings-inference` for TEI's OpenAI-compatible endpoint unless the local server requires a different model name.

Qdrant and embedding adapter failures are returned in `adapter_reports`. They do not block the local JSONL journal, so smoke tests can run without external credentials while production deployments can use standard vector/RAG services.

After Graphiti, Qdrant, or embedding credentials come online, replay the local journal into external adapters:

```sh
coat memory repair --file examples/memory-repair.json
```

Use `"dry_run": true` first to count selected records and adapter operations. Set `"store_kinds": ["qdrant"]` or `"store_kinds": ["zep_graphiti"]` to repair one adapter at a time.

## Live Agent Gates

Use stub sidecars until credentials and local daemons are configured.

- `CODEX_RUNNER_MODE=stub`
- `STAFF_ENGINEER_RUNNER_MODE=stub`
- `COAT_ALLOW_LOCAL_STUB_FALLBACK=true`

Compose sets `COAT_MEMORY_GATEWAY_URL=http://memory-gateway:9087` for both TypeScript sidecars. Each sidecar attempts a `memory_context` preflight during `/run-task`; if the memory gateway is unavailable, the sidecar still returns a result and includes `memory_context=unavailable` in diagnostics.

## Result Channels

Compose runs an S3-compatible MinIO object store:

- endpoint: `http://localhost:9000`
- console: `http://localhost:9001`
- bucket: `coat-artifacts`
- access key: `coat`
- secret key: `coat-local-secret`

Tasks that enable `ExecutionProfile.results.git` should return `git_result` with the task branch and worktree path. Tasks that enable `ExecutionProfile.results.object_storage` should return `object_artifacts` with `s3://...` URIs. Use git for code/diffs and object storage for large generated assets.

Do not enable live code execution without isolated workspaces and an explicit sandbox profile.

## Sandbox Runner

The sandbox runner listens on `http://localhost:9083` in Compose. It does not run arbitrary commands directly. It creates a deterministic per-task workspace, records a local registry file, writes `workspace-manifest.json`, and returns git/object-storage result refs that workers can use in their structured results.

Local Compose advertises only `SANDBOX_SUPPORTED_BACKENDS=local_workspace`. A request can declare `gvisor`, `kata`, `firecracker`, or `kubernetes_job`, but the local runner will return a `SandboxAttestation` warning unless the runner is deployed on infrastructure that can actually enforce that backend. Use `docs/design-docs/100-strong-sandboxing-guardrails.md` for the production pattern.

Snapshot and cleanup are idempotent:

```sh
coat sandbox plan --file examples/sandbox-workspace-request-gvisor.json
coat sandbox create --file examples/sandbox-workspace-request.json
coat sandbox create --file examples/sandbox-workspace-request-gvisor.json
coat sandbox snapshot --workspace-id <workspace-id>
coat sandbox cleanup --workspace-id <workspace-id>
```
