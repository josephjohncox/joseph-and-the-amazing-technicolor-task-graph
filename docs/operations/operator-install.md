# Operator Install Guide

This guide is the provider-neutral production installation path for Joseph and
the Amazing Technicolor Task Graph. It assumes the `coat` CLI is installed and
that release artifacts use the deployable `jattg` slug: Kubernetes namespace,
Helm chart, GHCR images, and release assets.

Use this guide for real clusters. Use `docs/operations/local-dev.md` for a
laptop-only smoke stack.

## Installation Profiles

Choose one profile before installing:

- Personal durable mode: local or small-cluster services with Restate Cloud
  tunnel, JSONL or Postgres projection, node-local agent auth, and private
  control-web access.
- Corporate managed mode: Kubernetes, Restate Cloud Operator or self-hosted
  Restate Operator, managed Postgres, managed object storage, Qdrant or approved
  vector memory, external secrets, OIDC front door, and isolated runner pools.
- Air-gapped or restricted mode: self-hosted Restate, private registry images,
  no public research adapters, local model endpoints, and preloaded memory or
  reference corpora.

The engine contract is the same in each profile: Restate owns workflow time,
the coordinator owns durable truth, runners do bounded work, and projections are
query surfaces.

## Prerequisites

- Kubernetes cluster with a dedicated `jattg` namespace.
- `coat`, `helm`, and `kubectl` installed on the operator workstation.
- Access to `ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/*`
  images, or mirrored images in a private registry.
- Restate Cloud environment plus signing public key, or a reviewed self-hosted
  Restate installation.
- Postgres database with `infra/db/migrations/` applied.
- S3-compatible object storage bucket for large artifacts.
- Secret management through Kubernetes Secrets, External Secrets, Vault, cloud
  secret manager, SOPS, 1Password, Bitwarden, or Doppler.
- Optional Qdrant plus Graphiti/Zep MCP memory services for durable semantic
  memory.

## Preflight

Render and inspect the main chart before touching a cluster:

```sh
coat deploy chart lint
coat deploy chart template --output /tmp/jattg.yaml
coat deploy cluster render --output infra/k8s/rendered.yaml
coat deploy cluster apply --file infra/k8s/rendered.yaml --dry-run=client
```

Check the operator values file for:

- image tags or digests;
- namespace, labels, and resource limits;
- `COAT_GOAL_STORE_BACKEND=postgres`;
- `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true`;
- Restate Cloud or self-hosted Restate endpoints;
- object storage endpoint, bucket, and workload identity or secret refs;
- runner registry, memory gateway, notifier, and event gateway URLs;
- OIDC front door and token-broker plan if this is multi-user.

## Install Core Services

Create the namespace and secrets with your normal secret-management path. Do not
commit the rendered Secret values from examples.

Install with Helm:

```sh
coat deploy chart upgrade \
  --values path/to/operator-values.yaml \
  --wait
```

Wait for the core services:

```sh
coat deploy cluster status --timeout 120s
```

Register the coordinator with Restate Cloud or the self-hosted Restate admin API
according to `docs/operations/restate-cloud.md`.

## Optional OIDC Dashboard Gateway

Single-user mode is the default. For team access, apply an OIDC front door:

```sh
coat deploy cluster apply \
  --file infra/k8s/examples/control-web-oidc-gateway.yaml \
  --dry-run=client
```

Edit issuer URL, redirect URL, host, TLS secret, client ID, client secret, and
cookie secret before removing `--dry-run=client`.

This gateway authenticates browser access to `control-web`. It is not the MCP
token broker. User-delegated MCP tasks still need `McpContextRef.access_mode =
multi_user_oidc`, a `UserPrincipalRef`, an `OidcDelegationPolicy`, consent or
approval refs, and runners labeled for `oidc_user_delegation`.

## Ephemeral Runners

Dynamic burst capacity should come from `ExecutionProfile.capacity`,
`ephemeralRunnerTemplates`, and the backend provisioner. Render static burst
runner examples only as fixtures or emergency manifests:

```sh
coat deploy cluster ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
coat deploy cluster ephemeral-jobs apply \
  --file infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml \
  --dry-run=client
```

Review namespace, NetworkPolicies, ServiceAccounts, injected env, image tags,
resource limits, and secrets before applying for real.

Production per-task Kubernetes execution should flow through the sandbox-runner
provisioner after budget and approval checks pass. Plan-only requests return the
Kubernetes objects; live requests use the Rust Kubernetes API path when
`SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`.

Prove this path on kind or k3d before using a managed cluster. The proof should
begin with a coordinator-approved capacity decision, then call the
sandbox-runner provision API. The sandbox-runner should perform server-side
dry-run first, then apply the bounded Job only after the approval and template
selection are recorded. The operator fixture commands below are for inspection
and emergency fallback; they are not the source of truth for live capacity.

For operator inspection, render a bounded Job from a `SandboxLaunchPlan`:

```sh
coat deploy cluster executor-job render \
  --launch-plan examples/sandbox-launch-plan-kubernetes-job.json \
  --output /tmp/jattg-executor-job.json
coat deploy cluster executor-job apply \
  --launch-plan examples/sandbox-launch-plan-kubernetes-job.json \
  --output /tmp/jattg-executor-job.json \
  --dry-run=client
```

Review the selected image, command, runtime class, service account, workspace
volume, resource limits, network policy labels, and plan ConfigMap before using
any manual apply path.

For the accepted proof, inspect the returned provision record instead of only
the rendered YAML. It should include the provision request ID, goal/task IDs,
capacity decision ref, applied ConfigMap and Job UIDs, selected Pod UID, watched
phase transitions, logs or log artifact refs, cleanup status, structured result
manifest refs, and sandbox attestation evidence. Treat a Pod `Succeeded` state
as necessary but not sufficient; task completion still requires the structured
result and attestation to be ingested by the coordinator and projected into the
goal store.

## Smoke Test

Submit a stub goal:

```sh
coat goal submit \
  --title "Cluster smoke" \
  --objective "Verify the durable control plane accepts and validates a bounded task."
coat goal progress --latest
coat runner status
coat store goals
```

Then inspect:

- `coat store tasks --goal-id <goal-id>`;
- `coat human notify --queue`;
- `coat event sources`;
- `coat memory health`;
- `control-web` through the private or OIDC-protected URL.

## Rollback

Rollback the chart:

```sh
coat deploy chart rollback --wait
```

Pause autonomous ingress before destructive maintenance:

- disable risky event sources or register them disabled;
- pause external schedulers and webhooks;
- stop burst runner Jobs;
- keep Restate workflow state and Postgres projections intact unless a human
  explicitly approves data deletion.

## Acceptance Checklist

- Restate request identity verification is configured.
- Postgres migrations are applied and `coat-goal-store` runs in Postgres mode.
- Object storage bucket and object refs are reachable from runners.
- Runner pools advertise only capabilities they can enforce.
- Default-deny NetworkPolicies are active for runner and sandbox namespaces.
- Control-web is private, VPN-restricted, or OIDC-protected.
- Event-source activation approval is enabled for risky source registration.
- Human feedback and approval notifications are visible in the notifier queue.
- `cargo test --workspace`, `buf lint`, `coat deploy chart lint`, and
  rendered manifest checks pass in CI.
