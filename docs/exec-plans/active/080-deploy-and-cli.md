# 080 Deploy And CLI

## Objective

Make the system operable through Compose, Kubernetes, and a CLI.

## Implementation

- Compose starts Restate, Rust services, TypeScript sidecars, and OpenTelemetry.
- Kubernetes manifests define Deployments, Services, ConfigMaps, Secrets, resource limits, and NetworkPolicies.
- CLI supports `init`, `goal submit`, `goal status`, `goal cancel`, `approve`, `compose up/down`, and `k8s render`.
- Add smoke examples under `examples/`.

## Tests

- `docker compose config` validates.
- `jattg k8s render` writes a manifest.
- `kubectl apply --dry-run=client` validates when available.
- Stub goal can be submitted to a running local Restate stack.

## Acceptance

- Operators can bring up the local stack from a clean checkout.
- Deployment artifacts do not require live agent credentials for smoke tests.
