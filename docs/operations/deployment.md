# Deployment Notes

## Compose

Compose is the default local deployment. It includes:

- Restate runtime
- Rust coordinator
- Rust runner registry
- Rust notifier
- Rust validator
- Rust sandbox runner
- Rust tool registry
- Codex runner sidecar
- Staff-engineer runner sidecar
- OpenTelemetry collector

## Kubernetes

Kubernetes manifests live in `infra/k8s/base/all.yaml`.

Expected production hardening:

- Add image digests.
- Replace placeholder secrets.
- Configure persistent Restate storage.
- Add namespace-specific resource requests and limits.
- Add per-task sandbox Jobs.
- Add ingress and TLS according to the target cluster.

## Health Checks

- Coordinator: `:9080/discover` through Restate SDK HTTP service discovery.
- Validator: `:9082/healthz`
- Sandbox runner: `:9083/healthz`
- Tool registry: `:9084/healthz`
- Runner registry: `:9085/healthz`
- Notifier: `:9086/healthz`
- Codex runner: `:9091/healthz`
- Staff-engineer runner: `:9092/healthz`
