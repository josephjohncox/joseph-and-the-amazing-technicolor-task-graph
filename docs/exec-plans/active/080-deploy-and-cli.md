# 080 Deploy And CLI

## Objective

Make the system operable through Compose, Kubernetes, and a CLI.

## Implementation

- Compose starts Restate, Rust services, a default multi-agent TypeScript runner pool, the TypeScript control gateway, and OpenTelemetry.
- Kubernetes manifests define Deployments, Services, ConfigMaps, Secrets, resource limits, and NetworkPolicies.
- Helm supports disabled-by-default ephemeral Job entries for bounded runner and executor capacity.
- Release packaging publishes `jattg-agent-toolbox` from the `agent-toolbox` Docker target while normal services keep the slim `service` target.
- CLI supports `coat k8s ephemeral-jobs render` and `coat k8s ephemeral-jobs apply` to materialize and apply the reusable bounded runner/executor Job example set without hand-copying manifests.
- CLI supports `init`, goal authoring/status/progress/tasks/steering/cancel, event source registration, runner registry, memory gateway, goal store, sandbox workspace lifecycle, local provider auth setup, chat-client MCP/skill setup, `approve`, `compose config/up/down`, Restate Cloud Compose env bootstrap and registration, and `k8s render/apply`.
- The optional control gateway exposes SPA and MCP views for goals, agent progress, prompts, human queues, events, runners, and memory without owning durable state.
- Add smoke examples under `examples/`.

## Tests

- `coat compose config` validates.
- `coat k8s render` writes a manifest.
- `coat k8s apply --dry-run=client` validates the base manifest when `kubectl` is available.
- `kubectl apply --dry-run=client` validates when available.
- Stub goal can be submitted to a running local Restate stack.
- `coat setup local-auth` starts an interactive provider setup wizard, while `--write-env`, `--check`, and `--print-commands` stay non-interactive for automation.
- `coat setup chat-client` starts an interactive MCP/skill setup wizard, while explicit install/write/print flags stay non-interactive for automation.

## Follow-Ups

- Keep Compose, Kubernetes, Helm, and release workflows aligned when service names, ports, secrets, or image names change.
- Add published binary and Helm chart smoke installs after the first GitHub Release is created.
- Add a production executor submit helper that creates per-task ephemeral Jobs from `SandboxLaunchPlan` once the Kubernetes executor adapter exists.

## Acceptance

- Operators can bring up the local stack from a clean checkout.
- Deployment artifacts do not require live agent credentials for smoke tests.
- Default Compose registers multiple stub runners with stable IDs so routing can exercise lane, role, and model selection locally.
- Primary chat clients can connect to the control gateway through MCP and a skill without becoming the durable coordinator.
- Kubernetes operators can use `jattg-agent-toolbox` for burst runners without replacing the slim always-on service images.
