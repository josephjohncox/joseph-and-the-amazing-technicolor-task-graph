# 130 Restate Cloud Personal And Corporate Deployment

## Objective

Make Restate Cloud a supported durable substrate for personal COAT usage and corporate managed deployments without changing the task-tree or runner contracts.

## Implementation

- Configure `coat-coordinator` to accept `RESTATE_IDENTITY_KEYS` and `RESTATE_SIGNING_PUBLIC_KEY` for Restate request identity verification.
- Add a Compose override that can run the Restate Cloud tunnel client and route event-triggered goals through the cloud ingress proxy.
- Add CLI helpers for printing cloud env, printing the tunnel Docker command, and registering the coordinator through a tunnel.
- Add Kubernetes examples for Restate Operator cloud registration and self-hosted Restate clusters.
- Document personal, public endpoint, and corporate deployment modes.
- Add provider-neutral operator install guidance covering Helm, Restate Cloud/self-hosted Restate, Postgres, object storage, OIDC front door, ephemeral runners, smoke tests, and rollback.

## Validation

- `cargo check -p coat-coordinator`
- `cargo check -p coat-cli`
- `coat compose config`
- `coat compose config --restate-cloud`
- CI verifies the Restate Cloud Compose profile with placeholder env.
- `coat restate register-cloud --dry-run`

## Follow-Ups

- Add provider-specific overlays after a concrete target such as EKS, GKE, AKS, k0s, k3s, or OpenShift is selected.
- Add service-level journal encryption guidance when the deployed SDK path supports it for Rust services.
