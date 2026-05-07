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
- Staff-engineer runner sidecar
- MinIO S3-compatible object store for local large-artifact refs
- OpenTelemetry collector

The Compose goal store listens on `:9088`, uses `COAT_GOAL_STORE_BACKEND=jsonl` by default, and replays `/data/goal-store.jsonl`. It stores both submitted goal projections and pre-goal durable planning-mode records. Set `COAT_GOAL_STORE_BACKEND=postgres` and `COAT_GOAL_STORE_DATABASE_URL=postgres://...` to use the standard Postgres read model instead.

The Compose event gateway listens on `:9089`, defaults to a JSONL journal, can switch to `COAT_EVENT_GATEWAY_BACKEND=postgres` for the SQL event inbox/outbox, and can submit generated goals to Restate through `COAT_RESTATE_INGRESS`.

The Compose control gateway listens on `:9090`. It reads goal-store projections, workflow status/progress, runner status, notifier threads, event gateway sources/triggers/events, and memory gateway results. Set `COAT_CONTROL_GATEWAY_TOKEN` to protect `/api/*` and `COAT_CONTROL_MCP_TOKEN` to protect `/mcp`.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` for production-like event-gateway deployments. With that switch enabled, risky enabled sources such as webhooks, calendars, schedules, or goal-creating routes require an approval reference at registration time. Use `coat event register --approval-id ...` or register proposed sources disabled first when the approval has not happened yet.

For personal Restate Cloud usage, use `infra/compose/docker-compose.restate-cloud.yml` with the `restate-cloud` profile. It starts the Restate Cloud tunnel client, configures coordinator request identity verification through `RESTATE_IDENTITY_KEYS` or `RESTATE_SIGNING_PUBLIC_KEY`, and points the event gateway at the tunnel ingress. See `docs/operations/restate-cloud.md`.

Postgres/pgvector is available as an optional profile for local operational-store development:

```sh
docker compose -f infra/compose/docker-compose.yml --profile db up postgres
COAT_GOAL_STORE_BACKEND=postgres docker compose -f infra/compose/docker-compose.yml --profile db up postgres goal-store
```

The profile mounts `infra/db/migrations/` into `/docker-entrypoint-initdb.d` on first boot. For production, run the same migrations with a real migration tool and managed credentials instead of relying on container init scripts.

## Kubernetes

Kubernetes manifests live in `infra/k8s/base/all.yaml`.

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
- Add namespace-specific resource requests and limits.
- Add per-task sandbox Jobs and set `runtimeClassName` for gVisor, Kata, Firecracker, or provider-integrated sandboxes when the node pool supports them.
- Use `infra/k8s/examples/sandbox-runtimeclasses.yaml` and `infra/k8s/examples/sandbox-task-pod.yaml` as starting points for RuntimeClass, Pod security context, and NetworkPolicy setup.
- Keep model-serving nodes separate from executor nodes when possible. See `docs/operations/model-runner-clusters.md` for GB10/DGX Spark, Mac mini, and mixed GPU/CPU runner fleets.
- Add ingress and TLS according to the target cluster.
- For Restate Cloud-backed clusters, prefer the Restate Operator `RestateCloudEnvironment` and `RestateDeployment` path in `infra/k8s/examples/restate-cloud-environment.yaml`.
- For self-hosted Restate clusters, use the operator pattern in `infra/k8s/examples/restate-operator-cluster.yaml` as the starting point and replace local storage with reviewed persistent or object-store-backed configuration.

## Auth Distribution

Production deployments should distribute auth through standard secret and identity systems:

- Kubernetes Secrets or External Secrets for static API tokens.
- Vault, AWS Secrets Manager, GCP Secret Manager, Azure Key Vault, 1Password, Bitwarden, Doppler, or SOPS-backed material for managed secret references.
- Kubernetes service accounts, cloud workload identity, Bedrock, Vertex, Foundry, or an LLM gateway for non-user service auth.
- External auth brokers for short-lived user-auth leases.

Codex and Claude Code device/browser sessions should normally stay node-local. Label those runners and route matching tasks to them. Do not replicate local login stores across nodes unless `allow_secret_sync=true`, the target store is encrypted, scope-limited, audited, and a human approval covers the distribution.

Brokered user auth should create a notification thread, ask the human to complete the login or device-code flow, store only a lease reference in coordinator-visible state, and expire the lease according to `AuthDistributionPolicy.lease_ttl_seconds`.

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
