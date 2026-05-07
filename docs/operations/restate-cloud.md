# Restate Cloud Operations

Restate Cloud is a first-class deployment target for personal COAT use and for teams that do not want to operate the durable journal themselves. COAT services still run where you choose: on a laptop, a private VM, Kubernetes, serverless, or a corporate cluster. Restate Cloud owns durable workflow execution, timers, invocation journals, and replay.

## Personal Cloud Mode

Use this mode when you want durable goals to keep progressing across local process restarts, laptop sleeps, or multiple personal runner nodes without exposing your coordinator publicly.

1. Create a Restate Cloud environment.
2. Copy the environment id, API key, region, and HTTP service signing public key from the Cloud UI.
3. Initialize the local Compose env file, fill in the values, validate the merged Compose config, then start the stack:

```sh
coat compose up --restate-cloud --init-env
# edit infra/compose/restate-cloud.env
coat compose config --restate-cloud
coat compose up --restate-cloud
```

`coat compose up --restate-cloud` uses `infra/compose/docker-compose.yml`,
`infra/compose/docker-compose.restate-cloud.yml`, the `restate-cloud` profile,
and `infra/compose/restate-cloud.env`. If the env file is missing, the CLI
copies `infra/compose/restate-cloud.env.example` and stops so secrets are not
silently defaulted. If placeholders remain, the CLI lists the keys to fill in.

The tunnel container maps Restate Cloud's ingress/admin proxies to host ports `18080` and `19070` by default so the local self-hosted Restate ports can still exist on `8080` and `9070`.

Start detached and register the coordinator through the tunnel in one command:

```sh
coat compose up --restate-cloud --register-cloud
```

`--register-cloud` implies detached Compose startup, then wraps `restate
deployments register --tunnel-name <name> <service-url>`. Use `--tunnel-name`
or `RESTATE_TUNNEL_NAME` if you changed the tunnel name from `jattg-personal`.
You can still call `coat restate register-cloud --dry-run` when you only want
to inspect the registration command.

Submit and inspect goals through the cloud ingress proxy:

```sh
COAT_RESTATE_INGRESS=http://localhost:18080 \
  coat goal submit --file examples/goal-template-structured.json
```

The coordinator validates Restate request identity when `RESTATE_IDENTITY_KEYS` or `RESTATE_SIGNING_PUBLIC_KEY` is set. Keep it set for any service reachable by Restate Cloud, including local tunnel scenarios.

## CLI Helpers

`coat restate cloud-env` prints the local environment exports COAT expects.

`coat restate tunnel-docker` prints an explicit `docker run` command for the official tunnel client when you do not want to use Compose.

`coat restate register-cloud` remains available as a lower-level helper and wraps:

```sh
restate deployments register --tunnel-name <name> <service-url>
```

Use `--dry-run` to inspect the command before executing it.

## Public Endpoint Mode

If the coordinator is deployed behind a public HTTPS endpoint, register that URL directly with Restate Cloud instead of using a tunnel. Public endpoints must use Restate request identity verification:

```sh
RESTATE_IDENTITY_KEYS=publickeyv1_... coat-coordinator
restate deployments register https://coordinator.example.com
```

Do not expose a coordinator without identity verification. Restate Cloud signs requests with an environment-specific key, and the SDK rejects calls that fail verification when the service is configured with the public key.

## Kubernetes With Restate Cloud

`coat compose` intentionally does not manage Kubernetes. Compose is the local
Docker lifecycle; cluster manifests and ephemeral runner Jobs live under
`coat k8s`, while packaged installs use the `infra/helm/jattg` chart. Use
`coat k8s render` and `coat k8s apply --dry-run=client` for raw manifest
workflows before moving to a real apply.

For clusters, prefer the Restate Operator path. The operator provides `RestateCloudEnvironment` and `RestateDeployment` resources so services can remain private and still register with Restate Cloud through an operator-managed tunnel.

Example manifests:

- `infra/k8s/examples/restate-cloud-environment.yaml`: cloud environment and coordinator `RestateDeployment` sketch.
- `infra/k8s/examples/restate-operator-cluster.yaml`: self-hosted Restate cluster sketch for teams that choose to operate Restate instead of using Cloud.

Use Kubernetes Secrets, External Secrets, Vault, cloud secret managers, or workload identity for API tokens. Do not bake Restate API keys into images or ConfigMaps.

## Personal Vs Corporate Defaults

Personal mode:

- Restate Cloud tunnel for coordinator reachability.
- Local Compose services and stub runners.
- Local JSONL goal/event stores unless Postgres is enabled.
- Node-local Codex or Claude device auth.
- MinIO for local object artifacts.

Corporate mode:

- Restate Cloud with operator-managed tunnels, or a self-hosted operator-managed Restate cluster.
- Managed Postgres for goal/event projections.
- Qdrant, pgvector, Zep/Graphiti, or another approved knowledge store for memory.
- Managed S3 or compatible object storage for artifacts.
- External Secrets, Vault, cloud secret managers, and workload identity.
- NetworkPolicies and separate runner pools for device-auth, API-key, local-model, and restricted-network tasks.

## Security Notes

- The tunnel exposes local Restate Cloud ingress/admin proxies. Treat access to those local proxy ports as equivalent to holding the configured Restate API key.
- `RESTATE_SIGNING_PUBLIC_KEY` is not a secret, but it is security-critical configuration.
- `RESTATE_BEARER_TOKEN` is a secret and must never be committed.
- Device-auth stores for Codex and Claude Code stay node-local by default. Distribute leases or labels, not raw login stores.
- Client-side journal encryption is currently SDK-specific; use it only after confirming the relevant SDK supports the needed codec for the deployed service.
