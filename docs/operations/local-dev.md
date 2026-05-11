# Local Development

## Build

The repo includes a committed `.envrc` for direnv. It adds the configured
checkout-local `coat` binary, `scripts/`, and local Node package bins to `PATH`,
then watches COAT config files for reloads. It does not export service endpoint
env vars. Endpoint defaults come from `.coat/project.json` and
`~/.coat/config.json`.

By default, `.envrc` matches `cargo build` and puts `target/debug` ahead of
`target/release` and any globally installed `coat`. Set
`COAT_BUILD_PROFILE=release` in `.envrc.local` only when this checkout should
prefer `target/release/coat`.

```sh
direnv allow
make build
coat guide --print
coat setup config --list-profiles
coat setup config --show
```

After the build, `coat` resolves from `COAT_BIN_DIR`. Copy
`.envrc.local.example` to `.envrc.local` only when a workstation should load
provider env files, tokens, release-profile build preference, or a rare
machine-local config override automatically.

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
npm run --prefix ui/control-plane-web smoke
```

## Protocol SDK Generation

`buf.gen.yaml` is the local generation scaffold for the future ProtocolSDK lane.
It keeps protobuf packages under `coat.v1`, writes generated Rust output to
`target/generated-sdks/rust`, and writes generated TypeScript output to
`target/generated-sdks/typescript`. Those paths are build artifacts, not
checked-in SDK packages.

```sh
make proto-sdk-generate
make proto-sdk-check
```

The selected SDK package names remain `coat`-scoped because the generated code
is an operator/control-plane API surface, not a Kubernetes chart or release
image. Use `coat-protocol-sdk` for the eventual Rust crate and
`@coat/protocol-sdk` for the eventual TypeScript package unless a later package
publishing decision supersedes the local scaffold. Keep `jattg` for Helm chart
names, Kubernetes objects, release archives, and published service images.

The current scaffold uses Buf remote plugins for `community/neoeinstein-prost`,
`community/neoeinstein-tonic`, and `bufbuild/es`. Generation may need network
access to the Buf Schema Registry the first time the plugins are resolved. The
Makefile runs generation with an isolated `BUF_GENERATE_HOME` under `target/` by
default, so a stale or invalid machine-level Buf token does not affect public
plugin resolution. Because this target resolves remote plugins, it is an
explicit ProtocolSDK validation step rather than part of the offline default
`make ci` path. Do not add generated files to source control until
compatibility rules, package metadata, and publish targets are selected.

## Local Deploy

```sh
coat deploy local config
coat deploy local preflight --allow-stub-runners
coat deploy local up --allow-stub-runners
coat deploy local config --profile db
coat deploy local up --allow-stub-runners --profile db
coat --config-profile restate-cloud deploy local up --restate-cloud --init-env
# edit infra/compose/restate-cloud.env
coat --config-profile restate-cloud deploy local config --restate-cloud
coat --config-profile restate-cloud deploy local up --restate-cloud --register-cloud --allow-stub-runners
```

`coat --config-profile restate-cloud deploy local up --restate-cloud` creates `infra/compose/restate-cloud.env`
from `infra/compose/restate-cloud.env.example` when missing and refuses to
start while placeholder Restate Cloud values remain. `--register-cloud` starts
Compose detached and then registers the coordinator through the default
`jattg-personal` tunnel.

Create a local provider env file when you want live hosted or local model credentials. The no-flag command opens an interactive setup wizard; the explicit flags are useful in CI, scripts, and copy-paste runbooks:

```sh
coat setup local-auth
coat setup login --codex --claude --preflight
coat setup sso --profile my-aws-sso-profile --write-env --bedrock-live --preflight
coat deploy local up --env-file infra/compose/local-providers.env
```

The setup wizard checks provider CLIs and environment variables without printing secret values, starts from the existing output env file when it exists, refreshes the models.dev catalog before rendering hosted model choices unless a cache is newer than 60 minutes, asks which provider surfaces to prepare, flips selected runner lanes from `stub` to `live`, and can write `infra/compose/local-providers.env` back with only the interactive changes applied. Existing auth modes, endpoints, model IDs, model params, memory-store URLs, and chat settings are used as prompt defaults instead of forcing operators to retype them. Codex runner setup is separate from OpenAI hosted model-provider setup. For Codex and Claude Code lanes it asks whether auth comes from env API keys, runner-local device/browser login, Codex App Server, or a brokered lease, so local smoke work is not blocked just because a static token is unavailable. For hosted model lanes it reads the external models.dev catalog from `COAT_MODEL_INDEX`, `.coat/model-index.json`, or `~/.coat/cache/models.dev.api.json`; `COAT_MODEL_INDEX` is treated as an explicit operator-managed catalog and `coat setup model-index refresh` remains available for manual cache warm-up. Selecting the OpenAI hosted surface writes `MODEL_PROVIDER_RUNNER_MODE=live` and can also write `MODEL_PROVIDER_RESEARCH_RUNNER_MODE=live`; if `OPENAI_API_KEY` is not set and no brokered auth mode is selected, preflight fails instead of silently leaving those lanes stubbed. Memory stores and embeddings stay disabled by default unless the wizard explicitly writes Qdrant, Graphiti/Zep MCP, embedding endpoint, and embedding model settings. For local model lanes and local embeddings it queries the configured OpenAI-compatible/Ollama endpoint for currently served models instead of using compiled-in defaults, then asks whether that same local model should also back the primary and research model-provider lanes. Runtime parameter choices remain indexed, including fast, speed-tier, fast-completions, balanced, deep-review, xhigh reasoning, deterministic JSON/tool-output, hosted chat, and custom escape hatches. Endpoint URLs remain editable text because they are deployment-specific. It can optionally copy already-exported secret values into that env file, but it never prompts for or prints secret values. When it writes the env file, it can immediately run selected `coat setup login` or `coat setup sso` actions and then run Compose preflight with that file. `scripts/coat-local-provider-setup.sh` is available as a checkout-local wrapper for machines that have not put `coat` on `PATH` yet.

`coat init` writes `.coat/project.json`, a non-secret project config with
standard `cli`, `local`, `restate-cloud`, and `eks` profiles. Most project
commands warn if that file is missing. `coat setup config --write-user` creates
a machine-local `~/.coat/config.json` template; use `coat --config-profile ...`
for one-off profile selection, and set `COAT_CONFIG` only when a machine should
use a non-default user config file. `coat deploy local up`
runs preflight before invoking Docker Compose: it checks the init marker,
Compose files, Docker availability, Restate Cloud env placeholders when enabled,
runner modes, and model/provider environment. An all-stub runner pool is allowed
only when `--allow-stub-runners` is explicit or a user config intentionally opts
into it.
See `docs/operations/configuration.md` for config layering and secret handling.

The Rust service image builds all `coat` Rust binaries once in a shared builder stage and copies the selected binary into each service image. It defaults to `CARGO_BUILD_JOBS=8` so builds still use parallel Rust compilation without multiplying compiler jobs across every Compose service.

Restate ingress is exposed on `http://localhost:8080`.
When using the Restate Cloud profile, cloud ingress is exposed through the tunnel on `http://localhost:18080` by default.
The coordinator service listens internally on `http://coordinator:9080`.
The control gateway and SPA listen on `http://localhost:9090`. Use the Chat tab to draft goals, plans, steering directives, and state explanations from plain language. Use the Memory tab to search context, write reviewed facts, join forked branch memories, retract or replace stale facts, dry-run adapter repair, inspect memory events, and apply sourced research output back into the goal as steering directives.
The default Compose stack starts a small multi-agent pool and auto-registers every runner with `runner-registry`:

- `codex-runner`: externally exposed coding lane on `localhost:9091`;
- `codex-reviewer-runner`: internal review/test/formal-methods lane;
- `claude-code-runner`: externally exposed Claude Code lane on `localhost:9094`;
- `model-provider-runner`: externally exposed generic hosted/local model lane on `localhost:9093`;
- `model-provider-research-runner`: internal research/review lane;
- `model-provider-local-runner`: internal host-local Ollama/vLLM-style lane;
- `staff-engineer-runner`: externally exposed issue-to-PR lifecycle lane on `localhost:9092`.

Use the internal-only runners through the registry and control gateway rather than direct host ports.
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
coat plan follow-ups
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
coat plan draft --file examples/plan-branch-from-existing.json
coat plan revise \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756710 \
  --file examples/plan-revision-branch-local-runners.json
coat plan compile \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756710 \
  --file examples/plan-compile-branch-new-goal.json \
  --out examples/drafts/local-model-runner-branch-goal.json
coat plan vote-candidate \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/plan-candidate-vote.json
coat plan select-candidate \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/plan-candidate-selection.json
coat setup local-auth
coat setup local-auth --write-env --output infra/compose/local-providers.env
coat setup chat-client
coat setup chat-client \
  --mcp-url http://localhost:9090/mcp \
  --install-codex-mcp \
  --install-codex-skill
coat setup chat-client \
  --mcp-url http://localhost:9090/mcp \
  --install-claude-mcp \
  --install-claude-skill
coat deploy local up --restate-cloud --allow-stub-runners
coat goal draft \
  --title "Local strict review smoke" \
  --objective "Create a strict-review goal draft with typed doctrine and a bounded initial frontier." \
  --strict-review \
  --human-steered \
  --out examples/drafts/local-strict-review.json
coat goal review-checks
coat goal submit --title "Smoke" --objective "Run a registered stub sidecar task"
export COAT_GOAL_ID=<goal-id-from-submit-output>
coat goal progress
coat goal list
coat goal submit --file examples/goal-template-structured.json
coat runner register --file examples/runner-vllm.json
coat runner list
coat runner status
coat runner dispatch --file examples/dispatch-smoke.json
coat runner register --file examples/runner-remote-codex.json
coat event register --file examples/event-source-calendar-schedule.json
coat event register --file examples/event-source-webhook-hmac.json
coat event register --file examples/event-source-generic-ci.json
coat event register --file examples/event-source-ide-lsp.json
coat event register --file examples/event-source-branch-activity.json
coat event register --file examples/event-source-pr-ci-failure.json
coat event register --file examples/event-source-sqs-notifications.json
coat event register --file examples/event-source-prometheus-alertmanager.json
coat event register --file examples/event-source-datadog-monitor.json
coat event register --file examples/event-source-slack-event.json
coat event register --file examples/event-source-stripe-webhook.json
coat event register \
  --file examples/event-source-webhook-hmac.json \
  --approval-id approval-123
coat store event-source-approvals --source-id github-review-webhook --limit 10
coat event ingest --file examples/external-event-calendar.json
coat event emit \
  --source-id ci-events \
  --file examples/generic-event-ci-failed.json
coat event emit \
  --source-id ide-lsp-diagnostics \
  --file examples/generic-event-ide-lsp-diagnostics.json
coat event emit \
  --source-id branch-activity \
  --file examples/generic-event-branch-updated.json
coat event emit \
  --source-id pr-ci-failures \
  --file examples/generic-event-pr-ci-failed.json
coat event webhook \
  --source-id prometheus-alertmanager \
  --file examples/prometheus-alertmanager-firing.json
coat event webhook \
  --source-id datadog-monitor \
  --file examples/datadog-monitor-alert.json
coat event poll-sqs --source-id sqs-notifications --max-messages 10
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
coat goal steer-standard \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756800 \
  --check behavioral_testing \
  --topic "end-to-end objective and operator workflow" \
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
coat human notify --file examples/notification-approval.json
coat human notify --file examples/notification-webhook.json
coat human notify --file examples/notification-dashboard.json
coat human notify --file examples/notification-email.json
coat human notify --file examples/notification-sqs.json
coat human notify --file examples/notification-pagerduty.json
coat human notify --file examples/notification-tracker-github.json
coat human notify --threads
coat human notify --thread-key local-model-coding-smoke
coat human notify --queue
coat store policy
coat store goals
coat store plans
coat store all-tasks
coat store approvals --limit 50
coat store record-artifacts --file examples/goal-store-record-artifacts.json
coat sandbox create --file examples/sandbox-workspace-request.json
coat sandbox create --file examples/sandbox-workspace-request-live-git.json
coat runner register --file examples/runner-local-tools.json
coat goal submit --file examples/goal-local-tool-execution.json
coat memory write --file examples/memory-write-fact.json
coat memory search --file examples/memory-search.json
coat memory join --file examples/memory-join.json
coat memory retract --file examples/memory-retract.json
coat memory preview-edit --file examples/memory-edit.json
coat memory edit --file examples/memory-edit.json
coat memory repair --file examples/memory-repair.json
coat memory events --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756602
coat deploy restate cloud-env
coat deploy restate register-cloud --dry-run
coat deploy cluster render --output infra/k8s/rendered.yaml
coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client
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
- `COAT_LLM_GATEWAY_URL`, `COAT_LLM_GATEWAY_API_KEY`, and
  `COAT_LLM_GATEWAY_{WORK,RESEARCH,CHAT,DEFAULT}_MODEL` for a shared
  OpenAI-compatible gateway such as Bifrost, LiteLLM, OpenRouter, Docker Model
  Gateway, or a private proxy
- `MODEL_PROVIDER_KIND`, `MODEL_PROVIDER_MODEL`, `MODEL_PROVIDER_ENDPOINT`, and `MODEL_PROVIDER_{LATENCY_CLASS,SPEED_TIER,TEMPERATURE,TOP_P,MAX_OUTPUT_TOKENS,REASONING_EFFORT,TIMEOUT_SECONDS}` for the generic provider runner
- `MODEL_PROVIDER_RESEARCH_KIND`, `MODEL_PROVIDER_RESEARCH_MODEL`, `MODEL_PROVIDER_RESEARCH_ENDPOINT`, and matching `MODEL_PROVIDER_RESEARCH_*` runtime params for the research-lane provider runner
- `LOCAL_MODEL_PROVIDER_KIND`, `LOCAL_MODEL_PROVIDER_MODEL`, `LOCAL_MODEL_PROVIDER_ENDPOINT`, and matching `LOCAL_MODEL_PROVIDER_*` runtime params for the host-local model runner
- `AWS_PROFILE`, `AWS_REGION`/`AWS_DEFAULT_REGION`, workload identity, or AWS credentials for Bedrock provider smoke tests

Sidecars also expose `GET /capabilities`, which is the quickest way to verify model candidates, review support, MCP propagation support, and remaining capacity without reading environment variables.

The runner registry records registrations and heartbeats in
`COAT_RUNNER_REGISTRY_JOURNAL_PATH` when configured. Compose sets this to
`/data/runner-registry.jsonl` on a named volume, so restarting
`runner-registry` preserves the visible runner set while stale heartbeat TTL and
capacity still decide whether a runner is dispatchable.

Run the bounded runner-registry smoke test without Docker or live model
credentials:

```sh
make runner-smoke
```

The smoke starts `coat-runner-registry` on an ephemeral localhost port with a
temporary journal, registers full, stale, and active runners through the HTTP
operator surface, checks `/runners/status`, verifies dispatch selects only the
active compatible remote runner, and asks `/capacity/plan` for a bounded scaling
recommendation.

Run the optional Compose-backed runner pool smoke when Docker is available:

```sh
make compose-runner-smoke
```

This uses `infra/compose/docker-compose.yml` with the existing
`runner-registry`, `codex-runner`, `codex-reviewer-runner`,
`claude-code-runner`, `model-provider-runner`,
`model-provider-research-runner`, `model-provider-local-runner`, and
`staff-engineer-runner` service names. The script forces those runner lanes to
stub mode for the smoke, starts an isolated Compose project, waits for sidecar
`/capabilities` responses, waits for all seven runner IDs to appear as
dispatchable in `/runners/status`, verifies `/dispatch` selects
`codex-runner-ts` for an explicit task contract, and verifies `/capacity/plan`
uses the heartbeat-derived pool supply.

If Docker, the daemon, or the Compose plugin is unavailable, the script prints a
clear `SKIP` line and exits successfully. CI jobs that require Docker coverage
can set `COAT_COMPOSE_RUNNER_SMOKE_REQUIRE_DOCKER=1` to turn that skip into a
failure. Set `COAT_COMPOSE_RUNNER_SMOKE_KEEP=1` to keep the temporary Compose
project for debugging.

Use sidecar verification endpoints for non-mutating dependency checks:

```sh
curl -sS http://localhost:9091/verify
curl -sS http://localhost:9092/verify
curl -sS http://localhost:9093/verify
curl -sS http://localhost:9094/verify
```

Codex MCP and App Server probes are opt-in through `CODEX_VERIFY_MCP=1` and `CODEX_VERIFY_APP_SERVER=1` so a basic health check never starts live agent infrastructure by accident.
Claude Code CLI probing is opt-in through `CLAUDE_CODE_VERIFY_CLI=1`. Model-provider endpoint probing is opt-in through `MODEL_PROVIDER_VERIFY_ENDPOINT=1`.

Local MCP smoke call:

```sh
coat tool list
coat tool call --name subagent_policy --file examples/tool-subagent-policy-request.json
```

Set `COAT_TOOL_REGISTRY_TOKEN` or `MCP_TOOL_TOKEN` when the registry requires
bearer auth for `/tools/list` and `/mcp`.
In Compose, `tool-registry` mounts the sandbox workspace volume read-only and sets `TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT=/workspaces`, so `artifact_manifest` can return `workspace-manifest.json`, `sandbox-launch-plan.json`, `snapshots/latest.json`, and worker `artifacts/artifact-manifest.json` for a `{goal_id, task_id}` lookup. Kubernetes should normally resolve large artifacts from object storage or the goal-store projection; set the same env var only when the registry can safely read the sandbox workspace volume.
Compose also sets `TOOL_REGISTRY_SANDBOX_RUNNER_URL=http://sandbox-runner:9083`. With that URL configured, the MCP `test_command` tool posts to `coat-sandbox-runner /commands/plan` and returns an approval-aware command plan instead of executing commands in the tool registry process.
The MCP `local_command` tool uses the same sandbox runner boundary for local binaries such as `git`, `docker`, `helm`, `kubectl`, build tools, and package managers. It plans by default; execution requires `execute=true`, an approval ID when `SANDBOX_REQUIRE_COMMAND_APPROVAL=true`, a known task workspace, and `SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION=true`.

For local Codex or Claude Code device/browser auth, prefer a runner-local setup:

- log in on the node that runs the sidecar with `coat setup login --codex --claude`; for Claude Code organization SSO or Console billing, use `--claude-sso`, `--claude-console`, and optionally `--claude-email`;
- set `CODEX_AUTH_MODE=runner_local_device`, `CLAUDE_CODE_AUTH_MODE=runner_local_device`, or `STAFF_ENGINEER_AUTH_MODE=runner_local_device` in `infra/compose/local-providers.env`;
- label that runner with values such as `auth.codex.device=true` or `auth.claude.device=true`;
- set task `auth_distribution.mode` to `runner_local_only`;
- keep `allow_secret_sync=false`.

Use `examples/auth-distribution-codex-device.json` for the node-local shape and `examples/auth-distribution-claude-brokered.json` for a brokered human-auth shape. Brokered user auth should emit an approval or feedback notification with the device-code URL or browser-login instructions, then resume through durable approval state.

Default local mode is single-user. Multi-user OIDC MCP delegation is opt-in and should stay off for local smoke runs unless you are testing an auth broker. Use `examples/mcp-context-multi-user-oidc.json`, set runner capabilities to include `oidc_user_delegation`, label the runner with `auth.oidc.user_delegation=true` and tenant labels, and keep all user tokens in the broker or MCP server. Task state should contain only `UserPrincipalRef`, `OidcDelegationPolicy`, consent refs, and `SecretRef` values.

The notifier stores local notification threads by `feedback_thread_key`, thread target address, or goal ID, and records each target delivery in a local outbox with `pending`, `delivered`, `awaiting_ack`, `acknowledged`, `retry_scheduled`, and `dead_lettered` states. This is for operator visibility in local runs; Restate workflow state remains the source of truth for approval and feedback signals. Set `COAT_NOTIFIER_JOURNAL_PATH` to persist the notifier thread and outbox journal across restarts. Use `COAT_NOTIFIER_MAX_ATTEMPTS` and `COAT_NOTIFIER_RETRY_BACKOFF_SECONDS` to tune local retry scheduling.
It supports `NotificationTargetKind::dashboard` for the dashboard human queue, `webhook` by posting the `NotificationRequest` JSON to the target address with optional `SecretRef` bearer auth, `slack` through an incoming webhook URL or secret ref, `email` as a structured local outbox, `sqs` through the official AWS SDK, `pager_duty` through Events API v2, and tracker webhook payloads for `git_hub`, `linear`, and `jira`. Set `COAT_EMAIL_OUTBOX_DIR` to persist email outbox messages as JSON files; otherwise they are only visible through notifier threads and `coat human notify --queue`.

For outbound SQS notifications, put the queue URL in `NotificationTarget.address` and let the SDK resolve credentials through normal AWS environment variables, profile configuration, IRSA, ECS task roles, or workload identity. Use `COAT_SQS_REGION`, `AWS_REGION`, or `AWS_DEFAULT_REGION` for region selection. Set `COAT_SQS_ENDPOINT_URL` for LocalStack or another SQS-compatible endpoint. FIFO queues get `COAT_SQS_MESSAGE_GROUP_ID` or the default `coat-notifications` group. A delivered SQS target that requires acknowledgement stays in `awaiting_ack` until an operator posts to `/outbox/{id}/ack`; failed targets move to `retry_scheduled` until `/outbox/{id}/retry` or `/outbox/retry-due` exhausts `COAT_NOTIFIER_MAX_ATTEMPTS`, after which they are visible through `/dlq`.

For inbound SQS events, register `examples/event-source-sqs-notifications.json`, edit the queue URL/region, then run `coat event poll-sqs --source-id sqs-notifications`. The event gateway polls with the same AWS SDK credential chain, converts each message body through the source `generic` JSON Pointer contract, records the normalized event, optionally routes it, and deletes the SQS message only when `sqs.delete_on_success=true` and ingest succeeds.

`coat-event-gateway` listens on `http://localhost:9089` in Compose. It records webhook, generic, calendar, scheduled, and bus events with dedupe keys, then can create or hold triggered goals. Set `COAT_EVENT_GATEWAY_TOKEN` to require bearer auth for mutating endpoints. Use `COAT_RESTATE_INGRESS` when the gateway should submit generated goals directly to Restate. When `COAT_GOAL_STORE_URL` is configured, trigger decisions with a concrete `goal_id` are also projected into the goal-store event read model so local operators can inspect the event-driven decision path beside goal history.

Prometheus Alertmanager and Datadog monitor examples are disabled by default but show the intended SRE, data-engineering, and data-science loop. When enabled, a provider webhook becomes an observability event with normalized fields under `payload._coat_observability`; the route creates a goal that first checks durable memory for recurrence before deciding whether to generate a service PR, dashboard or alert-tuning PR, data-quality investigation, SLO review, runbook update, or no-action report. Use `coat event webhook` for local JSON payload smoke tests; production providers should use webhook auth and event-source activation approval.

Local Compose defaults the event gateway to `COAT_EVENT_GATEWAY_BACKEND=jsonl`. To exercise the SQL event inbox/outbox path, start the database profile and set `COAT_EVENT_GATEWAY_BACKEND=postgres`; the service uses `COAT_EVENT_GATEWAY_DATABASE_URL`, falling back to the same Postgres database used by the goal store.

Generic sources use `GenericEventSource` to normalize arbitrary JSON or CloudEvents-compatible payloads. Register `examples/event-source-generic-ci.json`, then emit `examples/generic-event-ci-failed.json` with `coat event emit --source-id ci-events --file ...`. This is the default adapter for CI, git, issue tracker, chat, monitoring, database-change, memory, runner, and agent-topology events before a provider-specific adapter exists.

Webhook source auth is separate from gateway admin auth. Register a `WebhookAuthPolicy` with `kind=shared_secret_header`, `bearer_token`, or `hmac_sha256`, point `secret_ref` at an environment variable or local secret file for local smoke tests, and keep production secret providers behind Kubernetes, Vault, cloud secret stores, or workload identity. The example `examples/event-source-webhook-hmac.json` expects `COAT_GITHUB_WEBHOOK_SECRET`.
Provider presets cover GitHub HMAC, Slack Events API HMAC, and Stripe-style HMAC canonicalization. The Slack example expects `COAT_SLACK_SIGNING_SECRET`; the Stripe example expects `COAT_STRIPE_WEBHOOK_SECRET`.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` to smoke-test production activation policy. Risky enabled sources then need `coat event register --approval-id ...` on registration, or they should be registered with `"enabled": false` until a human approves activation. When `COAT_GOAL_STORE_URL` is configured on the event gateway, accepted activation references are projected to `coat-goal-store` and can be inspected with `coat store event-source-approvals`.

Run the local no-Docker event gateway smoke when you need ingress-to-projection
coverage without Compose:

```sh
make event-gateway-smoke
```

The smoke starts `coat-goal-store` and `coat-event-gateway` on ephemeral
localhost ports with temporary JSONL journals. It registers an approved risky
generic CI source, verifies the activation approval projects into goal-store,
emits and dedupes a generic event through `/events/generic/{source_id}`, checks
the normalized event fields, verifies the create-goal trigger is recorded when
Restate ingress is absent, queries the projected goal-store event for the
generated `goal_id`, and inspects both service journals. If the environment
cannot bind the needed localhost ports, it prints a clear `SKIP` line and exits
successfully.

The event API contract lives at `docs/api/event-gateway.asyncapi.yaml`. The cluster CronJob pattern lives at `infra/k8s/examples/calendar-trigger-cronjob.yaml`.

Run the optional LocalStack SQS EventOps proof when Docker is available:

```sh
make eventops-sqs-smoke
```

This starts a disposable LocalStack container with SQS only, creates
`coat-inbound-events` and `coat-notifications` queues, then starts
`coat-goal-store`, `coat-event-gateway`, and `coat-notifier` on ephemeral
localhost ports. The script derives local queue URLs from
`examples/event-source-sqs-notifications.json` and `examples/notification-sqs.json`
instead of changing those provider-neutral examples.

Inbound proof expectation: the script sends one generic notification-shaped JSON
message to `coat-inbound-events`, polls it through
`POST /events/sqs/sqs-notifications/poll`, verifies the gateway normalizes the
message into an `ExternalEvent`, routes it to human review, records the event and
trigger in the gateway journal, deletes the SQS message on success, and proves a
second poll receives zero messages.

Outbound proof expectation: the script posts the SQS notification example to
`coat-notifier`, verifies the notifier reports a delivered `sqs://message/...`
delivery, keeps the required-ack item visible in the local notifier queue with
an explicit outbox state, and
reads the outbound queue to confirm the durable notification envelope preserves
the request, target provider, event kind, feedback thread key, and ack
requirement.

If Docker, the Docker daemon, or LocalStack is unavailable, the script prints a
clear `SKIP` line and exits successfully. CI jobs that require this proof can set
`COAT_EVENTOPS_SQS_SMOKE_REQUIRE_LOCALSTACK=1` to turn those skips into failures.
The script defaults to `localstack/localstack:3.8.1` because `latest` can point
at auth-gated development images. Set `COAT_LOCALSTACK_IMAGE` to override that
image. This first proof covers basic inbound ack/delete and outbound delivery
with a journaled notifier outbox entry, while the notifier unit tests cover
local acknowledgement mutation, retry scheduling, journal replay, and DLQ state
transitions.

`coat-goal-store` listens on `http://localhost:9088` in Compose. It stores queryable goal, task, event, approval, and artifact projections in an append-only JSONL journal at `/data/goal-store.jsonl` by default. Restate remains authoritative; the goal store is for local operator inspection and future dashboards.

Inspect projections with:

```sh
coat store policy
coat store goal --goal-id <goal-id>
coat store tasks --goal-id <goal-id>
coat store events --goal-id <goal-id>
coat store artifacts --goal-id <goal-id>
coat store goal-approvals --goal-id <goal-id>
coat store event-source-approvals --limit 25
coat store record-artifacts --file examples/goal-store-record-artifacts.json
```

The web gateway uses the goal-store list endpoints for dashboard views:

```sh
curl -sS http://localhost:9090/api/plans
curl -sS http://localhost:9090/api/plans/<plan-id>/continuity
curl -sS http://localhost:9090/api/follow-ups
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

The Chat tab is always routed through the control gateway backend. The browser calls `/api/chat`; it does not call Ollama, vLLM, OpenAI, or other model providers directly.

Use the chat mode switcher for the three standard authoring paths: Plan, Goal, and Search. Search mode drafts a backend-routed search request and, when needed, a coordinator-owned research task proposal. It does not claim that memory, web, or reference search already ran unless a backend tool returned evidence.

While a chat request is running, open **Chat activity** in the SPA to inspect the operational trace: request mode, session, selected goal, backend resolution, model/stub stage, journaling status, and errors. This is an execution trace, not hidden model reasoning.

Chat history is durable server-side state. The gateway writes turns to
`coat-goal-store` by default (`COAT_CONTROL_CHAT_STORE_BACKEND=goal_store`), so
the same UI works with the goal-store JSONL journal in local Compose or the
Postgres read model in production. `COAT_CONTROL_CHAT_JOURNAL_PATH` is only a
gateway-local fallback for smoke tests or a temporarily unavailable goal-store.
The browser reads chat history through `/api/chat/session`; it does not own the
conversation log.

The gateway defaults to configured chat or the local stub so smoke tests do not need model credentials. It does not borrow local Ollama/vLLM runners just because they are registered. Runner lanes are selected by the coordinator from `MODEL_PROVIDER_*` and `LOCAL_MODEL_PROVIDER_*` based on task role, persona, labels, sandbox, and model route; the gateway Chat tab is selected separately from `COAT_CONTROL_CHAT_*`, `COAT_LLM_GATEWAY_*`, or direct OpenAI settings.

To force a specific live OpenAI-compatible chat backend, set:

```sh
COAT_CONTROL_CHAT_BACKEND=configured
COAT_CONTROL_CHAT_PROVIDER=openai_compatible
COAT_CONTROL_CHAT_COMPLETIONS_URL=http://localhost:8000/v1/chat/completions
COAT_CONTROL_CHAT_MODEL=<served-model>
COAT_CONTROL_CHAT_API_KEY=optional-provider-token
COAT_CONTROL_CHAT_SPEED_TIER=speed
COAT_CONTROL_CHAT_TEMPERATURE=0.2
COAT_CONTROL_CHAT_TOP_P=0.9
COAT_CONTROL_CHAT_MAX_OUTPUT_TOKENS=2048
COAT_CONTROL_CHAT_REASONING_EFFORT=low
COAT_CONTROL_CHAT_TIMEOUT_SECONDS=60
```

For OpenAI-hosted chat completions, set `COAT_CONTROL_CHAT_PROVIDER=openai`,
`COAT_CONTROL_CHAT_MODEL`, and `OPENAI_API_KEY` or
`COAT_CONTROL_CHAT_API_KEY`; the gateway will use
`https://api.openai.com/v1/chat/completions`. Runtime params are optional and
map to the OpenAI-compatible chat-completions request body. Chat drafts still
require explicit operator submission through the existing forms.

Runner-registry chat discovery is an advanced opt-in path:

```sh
COAT_CONTROL_CHAT_BACKEND=runner_registry
```

When enabled, discovery still requires an explicit runner or model label such as
`control_chat=true`, `chat.intent=user_request`, or
`routing_scope=operator_chat`. This is only operator-chat backend resolution for
a user request; it is not durable task dispatch.

Hosted-only request fields such as `reasoning_effort` and speed tier are only sent to hosted OpenAI chat endpoints. Local runner-discovered endpoints such as Ollama, vLLM, and llama.cpp receive portable sampling and token-limit parameters so they do not reject the request.

The service also has a real Postgres backend. Start the local database and goal store with:

```sh
COAT_GOAL_STORE_BACKEND=postgres \
  coat deploy local up --allow-stub-runners --profile db postgres goal-store
```

Keep full contract payloads in JSONB and use typed columns for IDs, statuses, roles, subgoals, event kinds, and artifact URIs. Migration scaffolding lives under `infra/db/migrations/`; Compose starts a local `pgvector/pgvector:pg16` database with the `db` profile.

Approval requests appear in goal state under `approvals` and in notifier threads when the task notification policy includes `approval_requested`. Approve or reject with:

```sh
coat human approve \
  --goal-id <goal-id> \
  --approval-id <approval-request-id> \
  --approved true
```

The default `ApprovalGatePolicy` requires approval for open network, non-isolated runners, secret-bearing MCP contexts, brokered user auth, dangerous MCP tools, high-risk local tool execution, privileged runner labels/capabilities, and any approval-policy `never` task that is not isolated.

Review output examples live under `examples/`. Runner implementations should return the same shape as `examples/review-output-changes-requested.json` when they need to block satisfaction and request an actor retry. Use `examples/review-output-doctrine-coverage.json` for formal-methods, type-soundness, hypothesis-testing, DDD, readability, abstraction, and security coverage. Findings should include file, line, and priority when the issue is tied to code; high/critical findings and priority 0/1 findings block validation even if the reviewer accidentally returns `accept`.

Tester and code-worker results should include `test_evidence` entries with command, exit code, pass/fail, duration, and stdout/stderr or artifact URIs whenever `done_criteria.tests_pass=true`. The validator treats missing or all-failing test evidence as incomplete for work-like and tester tasks.

Research output examples live under `examples/research-output-memory-substrate.json`. The proposed memory write in that fixture carries source-capture object refs for raw snapshots and fetch metadata, with SHA-256 digests, so reviewers can verify provenance without opening the network. `examples/web-search-response-replay.json` is the offline replay fixture for a routed research capture: it stores the original `WebSearchRequest`, the structured `AgentRunResult`, mirrored `ResearchOutput`, source artifacts, diagnostics, and the information-use plan so validators can exercise sourced research without live web access.

Validate replay capture locally with:

```sh
cargo test -p coat-domain research_replay_fixture_validates_without_live_web_access
cargo test -p coat-domain examples_parse_against_domain_contracts
```

A live research runner should replace stub sources with primary, official, or peer-reviewed sources and include a concrete information-use plan. When live adapters are added, commit captured replay fixtures before changing validator behavior so reviewers can reproduce citation checks without live web credentials.

Graphiti/Zep memory is configured as a policy and MCP endpoint in this scaffold. For local experiments, run the upstream Graphiti MCP server and point task MCP context at `http://localhost:8000/mcp/`; keep credentials in `SecretRef` or environment, not in goal JSON.

`coat-memory-gateway` listens on `http://localhost:9087` in Compose. Set `MEMORY_GATEWAY_TOKEN` to require bearer auth and `COAT_MEMORY_GATEWAY_TOKEN` for CLI calls.

Set `MEMORY_GATEWAY_JOURNAL_PATH` to enable the local append-only JSONL journal. Compose sets it to `/data/memory-gateway.jsonl` and mounts `memory-gateway-data`, so local memory records replay after service restart.

Set `MEMORY_GATEWAY_GRAPHITI_MCP_URL=http://localhost:8000/mcp/` when a Graphiti MCP server is running and the gateway should mirror local memory operations into the graph. `MEMORY_GATEWAY_GRAPHITI_GROUP_ID` defaults to `jattg`; per-goal `MemoryStoreRef.namespace` overrides it. Use `MEMORY_GATEWAY_GRAPHITI_TOKEN` only when the remote MCP endpoint requires bearer auth.

Compose runs Qdrant on `http://localhost:6333`, but `MEMORY_GATEWAY_QDRANT_URL` is blank until an operator enables the vector store. This keeps local smoke stacks on the JSONL journal unless the `coat setup local-auth` memory-store flow selects Qdrant. When Qdrant and an embedding endpoint/model are both configured, the gateway mirrors memory writes and joins into Qdrant, then merges vector hits into `memory_search`.

Live memory adapter tests are explicit opt-ins. `cargo test -p coat-memory-gateway live_qdrant_adapter_round_trips_when_enabled` runs only when `COAT_LIVE_QDRANT_MEMORY_TEST=true` and `MEMORY_GATEWAY_QDRANT_URL`, `MEMORY_GATEWAY_EMBEDDING_URL`, and `MEMORY_GATEWAY_EMBEDDING_MODEL` are set. `MEMORY_GATEWAY_EMBEDDING_DIMENSIONS` is required only when `MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS=true`; otherwise the live test uses the embedding vector length returned by the provider. `cargo test -p coat-memory-gateway live_graphiti_adapter_round_trips_when_enabled` runs only when `COAT_LIVE_GRAPHITI_MEMORY_TEST=true`, `COAT_LIVE_ZEP_GRAPHITI_MEMORY_TEST=true`, or `COAT_LIVE_ZEP_MEMORY_TEST=true` and `MEMORY_GATEWAY_GRAPHITI_MCP_URL` is set. Without those gates, the deterministic replay and JSONL tests pass without credentials.

Build a bounded worker context pack with:

```sh
coat memory context --file examples/memory-context.json
```

The response includes ranked memory hits, adapter reports, and an `InformationUsePlan` that tells the worker which facts are usable, which assumptions to avoid, and which validation checks to carry forward.

Embedding models are explicit:

- `coat setup local-auth` refreshes the models.dev cache before selecting hosted OpenAI embeddings unless the cache is newer than 60 minutes, so the wizard can read current embedding model IDs and dimensions. Run `coat setup model-index refresh` only when you want to warm or pin the cache separately.
- For Ollama, vLLM, llama.cpp, Hugging Face endpoints, TEI, or another OpenAI-compatible server, the wizard asks for the base endpoint, queries `/models` or Ollama tags, and writes the derived `/v1/embeddings` URL plus the selected served model ID.
- `MEMORY_GATEWAY_EMBEDDING_DIMENSIONS` is optional. Set `MEMORY_GATEWAY_EMBEDDING_SEND_DIMENSIONS=true` only when the selected provider expects a dimensions field.
- `OPENAI_API_KEY` or `MEMORY_GATEWAY_EMBEDDING_TOKEN` is required only for hosted or token-protected embedding endpoints.

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
- bucket: `jattg-artifacts`
- access key: `coat`
- secret key: `jattg-local-secret`

Tasks that enable `ExecutionProfile.results.git` should return `git_result` with the task branch and worktree path. Tasks that enable `ExecutionProfile.results.object_storage` should return `object_artifacts` with `s3://...` URIs. Use git for code/diffs and object storage for large generated assets.

The sandbox runner can validate the object-artifact contract locally without
contacting MinIO or live S3. When a workspace request enables `object_storage`
and provides an `ObjectStoreRef`, `coat-sandbox-runner` writes
`artifacts/artifact-manifest.json` with the planned artifact-manifest object,
the planned snapshot-manifest object, `upload_status=planned_external_upload`,
and `upload_performed_by_sandbox_runner=false`. `/snapshot` also writes
`snapshots/latest.json` with the planned snapshot upload ref. These manifests
are validation contracts for workers, validators, and a future uploader; they do
not mean bytes were uploaded.

Do not enable live code execution without isolated workspaces and an explicit sandbox profile.

## Sandbox Runner

The sandbox runner listens on `http://localhost:9083` in Compose. It creates a deterministic per-task workspace, records a local registry file, writes `workspace-manifest.json`, `sandbox-launch-plan.json`, `checkpoints/checkpoint-manifest.json`, and `artifacts/artifact-manifest.json`, and returns git/object-storage result refs that workers can use in their structured results.

Local binary execution is opt-in and allowlisted. `/commands/plan` is always available for approval-aware planning. `/commands/run` only executes bare binary names when `SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION=true`, the workspace exists, the command has an approval ID when required, and the binary is listed in `SANDBOX_ALLOWED_LOCAL_BINARIES`. When a request supplies task-local `ExecutionProfile.local_tools`, the sandbox runner also enforces denied binaries, allowed subcommands, denied arguments, network requirements, policy timeouts, and output limits before execution. The default allowlist is aimed at validation and operator tooling: `git`, build/test tools, package managers, `docker`, `helm`, and `kubectl`. Keep this disabled unless the runner node is isolated enough for the requested task.

Local Compose advertises only `SANDBOX_SUPPORTED_BACKENDS=local_workspace`. A request can declare `gvisor`, `kata`, `firecracker`, or `kubernetes_job`, but the local runner will return a `SandboxAttestation` warning unless the runner is deployed on infrastructure that can actually enforce that backend. Use `docs/design-docs/100-strong-sandboxing-guardrails.md` for the production pattern.

Snapshot and cleanup are idempotent:

```sh
coat sandbox plan --file examples/sandbox-workspace-request-gvisor.json
coat sandbox create --file examples/sandbox-workspace-request.json
coat sandbox create --file examples/sandbox-workspace-request-gvisor.json
coat sandbox snapshot --workspace-id <workspace-id>
coat sandbox cleanup --workspace-id <workspace-id>
```

## Ephemeral Kubernetes Runner Jobs

Ephemeral capacity is a coordinator/executor policy path. In local docs the
static manifests are fixtures for review; production should use
`ExecutionProfile.capacity` and the backend provisioner.

Render the reusable bounded Job example set from the CLI when you need a fixture:

```sh
coat deploy cluster ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
coat deploy cluster ephemeral-jobs apply \
  --file infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml \
  --dry-run=client
```

The rendered file contains default-deny network policy, a restricted service
account, injection ConfigMap, model-provider runner, Claude Code runner, and a
temporary Restate executor pattern using the `jattg-agent-toolbox` image.

For a single per-task executor, first obtain a launch plan from the sandbox
runner. The backend provisioner endpoint can plan the ConfigMap and Job objects:

```sh
curl -sS -X POST http://localhost:9083/kubernetes/executor-jobs/provision \
  -H 'content-type: application/json' \
  --data @examples/kubernetes-executor-job-provision.json
```

The CLI renderer remains available for inspection:

```sh
coat sandbox plan --file examples/sandbox-workspace-request-gvisor.json > /tmp/sandbox-launch-plan.json
coat deploy cluster executor-job render \
  --launch-plan /tmp/sandbox-launch-plan.json \
  --output /tmp/jattg-executor-job.json
```

Both paths produce Kubernetes objects containing a ConfigMap with
`sandbox-launch-plan.json` and a Job that projects runtime class, resources,
network labels, security context, environment, and command from the plan. Set
`SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true` and request `server_dry_run` or
`apply` only when the sandbox runner should contact a real Kubernetes control
plane.
