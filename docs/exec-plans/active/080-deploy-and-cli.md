# 080 Deploy And CLI

## Objective

Make the system operable through Compose, Kubernetes, and a CLI.

## Implementation

- Compose starts Restate, Rust services, TypeScript runner sidecars, the TypeScript control gateway, and OpenTelemetry.
- Kubernetes manifests define Deployments, Services, ConfigMaps, Secrets, resource limits, and NetworkPolicies.
- Helm supports disabled-by-default ephemeral Job entries for bounded runner and executor capacity.
- Release packaging publishes `jattg-agent-toolbox` from the `agent-toolbox` Docker target while normal services keep the slim `service` target.
- CLI supports `coat k8s ephemeral-jobs render` to materialize the reusable bounded runner/executor Job example set without hand-copying manifests.
- CLI supports `init`, goal authoring/status/progress/tasks/steering/cancel, event source registration, runner registry, memory gateway, goal store, sandbox workspace lifecycle, `approve`, `compose up/down`, and `k8s render`.
- The optional control gateway exposes SPA and MCP views for goals, agent progress, prompts, human queues, events, runners, and memory without owning durable state.
- Add smoke examples under `examples/`.

## Tests

- `docker compose config` validates.
- `coat k8s render` writes a manifest.
- `kubectl apply --dry-run=client` validates when available.
- Stub goal can be submitted to a running local Restate stack.

## Follow-Ups

- Keep Compose, Kubernetes, Helm, and release workflows aligned when service names, ports, secrets, or image names change.
- Add published binary and Helm chart smoke installs after the first GitHub Release is created.
- Add a submit/apply helper for ephemeral runner Job templates once the production Kubernetes executor adapter exists.

## Acceptance

- Operators can bring up the local stack from a clean checkout.
- Deployment artifacts do not require live agent credentials for smoke tests.
- Kubernetes operators can use `jattg-agent-toolbox` for burst runners without replacing the slim always-on service images.
