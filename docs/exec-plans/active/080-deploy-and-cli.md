# 080 Deploy And CLI

## Objective

Make the system operable through Compose, Kubernetes, and a CLI.

## Implementation

- Compose starts Restate, Rust services, a default multi-agent TypeScript runner pool, the TypeScript control gateway, and OpenTelemetry.
- Kubernetes manifests define Deployments, Services, ConfigMaps, Secrets, resource limits, and NetworkPolicies.
- Helm supports capacity templates for coordinator/executor provisioners plus disabled-by-default manual ephemeral Job entries as escape hatches.
- Release packaging publishes `jattg-agent-toolbox` from the `agent-toolbox` Docker target while normal services keep the slim `service` target.
- CLI supports `coat deploy cluster ephemeral-jobs render` and `coat deploy cluster ephemeral-jobs apply` as fixture and emergency manifest paths, while the backend provisioner is the normal capacity path.
- CLI supports `coat deploy cluster executor-job render/apply` as an operator inspection path; `coat-sandbox-runner` owns the Rust Kubernetes API provisioning path for per-task executor Jobs.
- CLI uses a canonical workflow hierarchy: `plan`, `goal`, `human`, `deploy`, `runner`, `memory`, `event`, `store`, `sandbox`, `release`, and `setup`.
- Bare `coat` prints root help; `coat guide` is limited to setup, human queue, approval, chat-client, and follow-up workflows where dialogue is useful.
- `coat human approve/notify` owns human feedback; `coat deploy local/cluster/chart/restate` owns Compose, Kubernetes, Helm, and Restate Cloud operations without duplicate top-level command groups.
- The optional control gateway exposes SPA and MCP views for goals, agent progress, prompts, human queues, events, runners, and memory without owning durable state.
- Add smoke examples under `examples/`.

## Tests

- `coat deploy local preflight --allow-stub-runners` validates intentional stub smoke stacks.
- `coat deploy local config` validates merged Compose configuration.
- `coat deploy cluster render` writes a manifest.
- `coat deploy cluster apply --dry-run=client` validates the base manifest when `kubectl` is available.
- `coat deploy cluster status` wraps the core deployment rollout checks after install.
- `coat deploy cluster executor-job render` projects a `SandboxLaunchPlan` into a Job manifest without requiring live cluster access, and sandbox-runner provisioner tests cover the backend Job object projection.
- `coat deploy chart lint` and `coat deploy chart template --output <file>` validate the chart when `helm` is available.
- Stub goal can be submitted to a running local Restate stack.
- `coat setup local-auth` starts an interactive provider setup wizard with provider, model, and runtime-param selectors, while `--write-env`, `--check`, and `--print-commands` stay non-interactive for automation.
- `coat setup local-auth` reads the existing provider env file before prompting and uses configured auth modes, endpoints, model IDs, runtime params, memory stores, embeddings, and Chat settings as overrideable interactive defaults.
- `coat setup local-auth` auto-refreshes the external models.dev catalog before hosted selectors render, debounces refreshes for 60 minutes, preserves explicit `COAT_MODEL_INDEX` catalogs, and model/embedding selection tests prove hosted choices come from that catalog or live served-model discovery instead of compiled-in IDs.
- `coat deploy local preflight` rejects embedding endpoints without model IDs and warns when Qdrant is configured without an active embedding pair.
- `coat setup login` and `coat setup sso` run provider login/setup flows and optional Compose preflight without making operators copy raw provider commands from docs.
- `coat setup chat-client` starts an interactive MCP/skill setup wizard, while explicit install/write/print flags stay non-interactive for automation.
- Bare `coat` shows root help, and `coat guide --print` shows the canonical command map.
- `coat deploy local preflight` blocks uninitialized or accidentally all-stub runs unless the operator passes the explicit allow flag.

## Follow-Ups

- Keep Compose, Kubernetes, Helm, and release workflows aligned when service names, ports, secrets, or image names change.
- Add published binary and Helm chart smoke installs after the first GitHub Release is created.
- Add a production controller/provisioner loop that submits per-task executor Jobs from coordinator-approved state and records completion attestations.

## Acceptance

- Operators can bring up the local stack from a clean checkout.
- Deployment artifacts do not require live agent credentials for smoke tests.
- Default Compose registers multiple stub runners with stable IDs so routing can exercise lane, role, and model selection locally.
- Primary chat clients can connect to the control gateway through MCP and a skill without becoming the durable coordinator.
- Kubernetes operators can use `jattg-agent-toolbox` for burst runners without replacing the slim always-on service images.
