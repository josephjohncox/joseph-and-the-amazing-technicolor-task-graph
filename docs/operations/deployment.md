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
- Codex runner sidecar
- Staff-engineer runner sidecar
- MinIO S3-compatible object store for local large-artifact refs
- OpenTelemetry collector

The Compose goal store listens on `:9088`, uses `COAT_GOAL_STORE_BACKEND=jsonl` by default, and replays `/data/goal-store.jsonl`. Set `COAT_GOAL_STORE_BACKEND=postgres` and `COAT_GOAL_STORE_DATABASE_URL=postgres://...` to use the standard Postgres read model instead.

The Compose event gateway listens on `:9089`, uses a JSONL journal, and can submit generated goals to Restate through `COAT_RESTATE_INGRESS`.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` for production-like event-gateway deployments. With that switch enabled, risky enabled sources such as webhooks, calendars, schedules, or goal-creating routes require an approval reference at registration time. Use `coat event register --approval-id ...` or register proposed sources disabled first when the approval has not happened yet.

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
- Replace the JSONL event gateway backend with a Postgres event inbox/outbox, or bridge high-volume channels to Kafka, Redpanda, NATS, SQS/SNS, Pub/Sub, or EventBridge.
- Use Kubernetes CronJobs for detached scheduled triggers and Restate timers for durable waits inside active workflows.
- Use `infra/k8s/examples/calendar-trigger-cronjob.yaml` as the cluster pattern for scheduled event generation, and keep it suspended until a source route and auth policy are approved.
- Use `infra/k8s/examples/postgres-pgvector.yaml` only as a development StatefulSet pattern; production clusters should prefer managed Postgres or a reviewed operator.
- Use managed S3 in AWS/EKS, or a production object-store service, for `ObjectStoreRef` artifacts.
- Replace local MinIO root credentials and prefer workload identity or External Secrets.
- Add namespace-specific resource requests and limits.
- Add per-task sandbox Jobs.
- Add ingress and TLS according to the target cluster.

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
- Codex runner: `:9091/healthz`
- Staff-engineer runner: `:9092/healthz`
