# Deployment Notes

## Compose

Compose is the default local deployment. It includes:

- Restate runtime
- Rust coordinator
- Rust event gateway
- Rust goal store projection service
- Rust runner registry
- Rust notifier
- Rust validator
- Rust sandbox runner
- Rust tool registry
- TypeScript control gateway and SPA
- Codex runner sidecar
- Codex reviewer/tester runner sidecar
- Claude Code runner sidecar
- Model-provider runner sidecar
- Model-provider research runner sidecar
- Host-local model-provider runner sidecar
- Staff-engineer runner sidecar
- MinIO S3-compatible object store for local large-artifact refs
- OpenTelemetry collector

The Compose goal store listens on `:9088`, uses `COAT_GOAL_STORE_BACKEND=jsonl` by default, and replays `/data/goal-store.jsonl`. It stores both submitted goal projections and pre-goal durable planning-mode records. Set `COAT_GOAL_STORE_BACKEND=postgres` and `COAT_GOAL_STORE_DATABASE_URL=postgres://...` to use the standard Postgres read model instead.

The Compose event gateway listens on `:9089`, defaults to a JSONL journal, can switch to `COAT_EVENT_GATEWAY_BACKEND=postgres` for the SQL event inbox/outbox, and can submit generated goals to Restate through `COAT_RESTATE_INGRESS`.

The Compose control gateway listens on `:9090`. It reads goal-store projections, workflow status/progress, runner status, notifier threads, event gateway sources/triggers/events, and memory gateway results. It can also draft goals/plans/steering through the chat assistant, submit memory join/retract/edit/repair commands, and convert sourced research output into `GoalWorkflow/steer` directives. Set `COAT_CONTROL_GATEWAY_TOKEN` to protect `/api/*` and `COAT_CONTROL_MCP_TOKEN` to protect `/mcp`.

The default Compose runner pool intentionally has multiple task lanes. Primary runner ports are exposed for local inspection, while additional review/research/local-model lanes are internal-only and selected through `coat-runner-registry`. Use the interactive `coat setup local-auth` wizard to create the env file for hosted provider keys, Bedrock/AWS routing, local Ollama/vLLM endpoints, and control-gateway chat model settings. Use `coat setup local-auth --write-env --output infra/compose/local-providers.env` when automation needs the non-interactive template path. Use `coat compose config` and `coat compose up` for the normal local lifecycle.

Compose defaults to single-user mode. Multi-user OIDC MCP delegation is an extension path and requires an external OIDC-aware gateway or broker; do not enable user-delegated MCP auth by sharing local browser or CLI tokens.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` for production-like event-gateway deployments. With that switch enabled, risky enabled sources such as webhooks, calendars, schedules, or goal-creating routes require an approval reference at registration time. Use `coat event register --approval-id ...` or register proposed sources disabled first when the approval has not happened yet.

For personal Restate Cloud usage, prefer `coat compose up --restate-cloud`.
It uses `infra/compose/docker-compose.restate-cloud.yml` with the
`restate-cloud` profile, creates the local env file from the example when
missing, blocks placeholder values, starts the Restate Cloud tunnel client,
configures coordinator request identity verification through
`RESTATE_IDENTITY_KEYS` or `RESTATE_SIGNING_PUBLIC_KEY`, and points the event
gateway at the tunnel ingress. Use `coat compose up --restate-cloud
--register-cloud` when you want detached startup plus coordinator registration.
See `docs/operations/restate-cloud.md`.

Postgres/pgvector is available as an optional profile for local operational-store development:

```sh
coat compose up --profile db postgres
COAT_GOAL_STORE_BACKEND=postgres coat compose up --profile db postgres goal-store
```

The profile mounts `infra/db/migrations/` into `/docker-entrypoint-initdb.d` on first boot. For production, run the same migrations with a real migration tool and managed credentials instead of relying on container init scripts.

## Kubernetes

Kubernetes manifests live in `infra/k8s/base/all.yaml`.

Keep Kubernetes under `coat k8s`, not `coat compose`. The commands share the
same service-boundary assumptions, but they target different runtimes:
Compose starts local containers; `coat k8s render` and `coat k8s
ephemeral-jobs apply` materialize and apply cluster manifests; Helm installs
the packaged `infra/helm/jattg` chart.

Render, validate, and apply the base manifest through the CLI:

```sh
coat k8s render --output infra/k8s/rendered.yaml
coat k8s apply --file infra/k8s/rendered.yaml --dry-run=client
coat k8s apply --file infra/k8s/rendered.yaml --namespace jattg
```

Expected production hardening:

- Add image digests.
- Replace placeholder secrets.
- Configure persistent Restate storage.
- Set `COAT_GOAL_STORE_BACKEND=postgres` and provide `COAT_GOAL_STORE_DATABASE_URL` from a Secret, External Secret, Vault, cloud secret manager, or workload identity path.
- Use the Postgres event inbox/outbox for durable operator-scale ingress, or bridge high-volume channels to Kafka, Redpanda, NATS, SQS/SNS, Pub/Sub, or EventBridge.
- Use Kubernetes CronJobs for detached scheduled triggers and Restate timers for durable waits inside active workflows.
- Use `infra/k8s/examples/calendar-trigger-cronjob.yaml` as the cluster pattern for scheduled event generation, and keep it suspended until a source route and auth policy are approved.
- Use `infra/k8s/examples/postgres-pgvector.yaml` only as a development StatefulSet pattern; production clusters should prefer managed Postgres or a reviewed operator.
- Use managed S3 in AWS/EKS, or a production object-store service, for `ObjectStoreRef` artifacts.
- Replace local MinIO root credentials and prefer workload identity or External Secrets.
- Put `control-web` behind ingress/TLS, OAuth proxy, VPN, or private network access before exposing it outside the cluster.
- For multi-user deployments, put `control-web` behind OIDC-aware ingress or an auth gateway, configure a token broker, and use `McpContextRef.access_mode=multi_user_oidc` only for goals that need user-delegated MCP calls.
- Add namespace-specific resource requests and limits.
- Use namespace default-deny NetworkPolicies for runner and sandbox namespaces, then allow only DNS, coordinator/registry/memory/object-store, approved model endpoints, approved research gateways, and Restate service registration paths needed by that runner profile.
- Add per-task sandbox Jobs and set `runtimeClassName` for gVisor, Kata, Firecracker, or provider-integrated sandboxes when the node pool supports them.
- Use `infra/k8s/examples/sandbox-runtimeclasses.yaml` and `infra/k8s/examples/sandbox-task-pod.yaml` as starting points for RuntimeClass, Pod security context, and NetworkPolicy setup.
- Keep model-serving nodes separate from executor nodes when possible. See `docs/operations/model-runner-clusters.md` for GB10/DGX Spark, Mac mini, and mixed GPU/CPU runner fleets.
- Use `infra/k8s/examples/ephemeral-agent-runner-jobs.yaml` and `docs/operations/ephemeral-kubernetes-runners.md` for burst runner Jobs, short-lived Claude Code/Codex/model-provider runners, and temporary Restate service executors.
- Add ingress and TLS according to the target cluster.
- For multi-user dashboard access, use `infra/k8s/examples/control-web-oidc-gateway.yaml` as an OAuth2 Proxy front-door example. It authenticates the SPA/control gateway without changing the Rust engine and keeps COAT in single-user mode until a goal explicitly uses `McpContextRef.access_mode=multi_user_oidc`.
- For Restate Cloud-backed clusters, prefer the Restate Operator `RestateCloudEnvironment` and `RestateDeployment` path in `infra/k8s/examples/restate-cloud-environment.yaml`.
- For self-hosted Restate clusters, use the operator pattern in `infra/k8s/examples/restate-operator-cluster.yaml` as the starting point and replace local storage with reviewed persistent or object-store-backed configuration.

## Helm

The Helm chart lives in `infra/helm/jattg`. It follows the same logical service boundaries as `infra/k8s/base/all.yaml`, but is values-driven for release installation.

The chart also supports disabled-by-default `.Values.ephemeralJobs` entries.
Use them for bounded runner or executor Jobs that run the `jattg-agent-toolbox`
image, register with the runner registry or Restate, and terminate by
`activeDeadlineSeconds` plus `ttlSecondsAfterFinished`.
`infra/helm/jattg/values-ephemeral-example.yaml` renders a model-provider burst
runner example against the same chart.
For raw manifest workflows, render the reusable example Jobs with:

```sh
coat k8s ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
coat k8s ephemeral-jobs apply \
  --file infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml \
  --dry-run=client
```

Drop `--dry-run=client` only after reviewing the rendered namespace,
NetworkPolicies, ServiceAccounts, Secrets, runner image tags, resource limits,
and any injected environment references for the target cluster.

Local validation:

```sh
helm lint infra/helm/jattg
helm template jattg infra/helm/jattg > /tmp/jattg-helm.yaml
```

Package locally:

```sh
CHART_VERSION=0.2.0 APP_VERSION=0.2.0 scripts/package-helm-chart.sh
```

GitHub chart releases are separate from binary releases. See `docs/operations/releases.md`.

## Auth Distribution

Production deployments should distribute auth through standard secret and identity systems:

- Kubernetes Secrets or External Secrets for static API tokens.
- Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault, 1Password, Bitwarden, Doppler, or SOPS-backed material for managed secret references.
- Kubernetes service accounts, cloud workload identity, Bedrock, Vertex, Foundry, or an LLM gateway for non-user service auth.
- External auth brokers for short-lived user-auth leases.
- OIDC token brokers for user-delegated MCP access, using on-behalf-of or token-exchange flows with per-MCP audience, scope, TTL, tenant, and consent checks.

Codex and Claude Code device/browser sessions should normally stay node-local. Label those runners and route matching tasks to them. Do not replicate local login stores across nodes unless `allow_secret_sync=true`, the target store is encrypted, scope-limited, audited, and a human approval covers the distribution.

Brokered user auth should create a notification thread, ask the human to complete the login or device-code flow, store only a lease reference in coordinator-visible state, and expire the lease according to `AuthDistributionPolicy.lease_ttl_seconds`.

OIDC-backed MCP user delegation follows the same principle: store `UserPrincipalRef`, `OidcDelegationPolicy`, consent refs, and `SecretRef` broker refs, but never raw user ID tokens, access tokens, refresh tokens, cookies, or browser sessions. Runners must advertise `oidc_user_delegation` and tenant labels before they can receive such tasks.

## Health Checks

- Coordinator: `:9080/discover` through Restate SDK HTTP service discovery.
- Validator: `:9082/healthz`
- Sandbox runner: `:9083/healthz`
- Tool registry: `:9084/healthz`
- Runner registry: `:9085/healthz`
- Notifier: `:9086/healthz`
- Goal store: `:9088/healthz`
- Event gateway: `:9089/healthz`
- Control gateway: `:9090/healthz`
- Codex runner: `:9091/healthz`
- Staff-engineer runner: `:9092/healthz`
- Model-provider runner: `:9093/healthz`
- Claude Code runner: `:9094/healthz`
