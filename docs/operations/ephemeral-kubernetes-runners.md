# Ephemeral Kubernetes Runners

Use ephemeral Kubernetes Jobs when a goal needs burst runner capacity, a
time-boxed local-model worker, a short-lived Claude Code/Codex runner, or a
temporary Restate service executor. Restate and the coordinator still own
durable state; Jobs only provide execution capacity.

## Image Model

The default image is `jattg-agent-toolbox`, built from the `agent-toolbox` target
in `infra/containers/rust-service.Dockerfile`.

It includes:

- all Rust service binaries and the `coat` CLI;
- built TypeScript runner sidecars for Codex, Claude Code, model-provider, and staff-engineer;
- common operator tools: `bash`, `curl`, `git`, `jq`, `ripgrep`, `openssh-client`, Python, build tools, Node, and npm;
- `/usr/local/bin/jattg-ephemeral-entrypoint`, which dispatches by `COAT_EPHEMERAL_KIND`.

Supported `COAT_EPHEMERAL_KIND` values:

- `codex-runner`
- `claude-code-runner`
- `model-provider-runner`
- `staff-engineer-runner`
- `coordinator`
- `event-gateway`
- `goal-store`
- `memory-gateway`
- `notifier`
- `runner-registry`
- `sandbox-runner`
- `tool-registry`
- `validator`
- `command`

Use the slim service images for normal always-on Deployments. Use
`jattg-agent-toolbox` for ephemeral Jobs where startup simplicity and tooling
coverage matter more than image size.

## Injection

The toolbox image has a controlled injection surface at
`/opt/coat/injections`:

- `env/*.env`: sourced before the selected process starts;
- `bin/`: prepended to `PATH`;
- `init.d/*.sh`: executed only when `COAT_ENABLE_INJECTION_SCRIPTS=true`.

Mount ConfigMaps for non-secret scripts/config and Secrets for credentials.
Do not place raw credentials in task state, runner registration, diagnostics, or
memory. Prefer workload identity, External Secrets, Vault, cloud secret stores,
or short-lived broker leases.

## Runner Jobs

`infra/k8s/examples/ephemeral-agent-runner-jobs.yaml` includes examples for:

- a model-provider runner Job that can point at vLLM, Ollama, Bedrock, or an
  OpenAI-compatible gateway;
- a Claude Code runner Job with node-local or secret-backed auth;
- a temporary coordinator Restate service executor with a Service and
  registration Job.

Render the reusable raw manifest set from the CLI:

```sh
coat k8s ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
```

Runner Jobs should:

- set `activeDeadlineSeconds` and `ttlSecondsAfterFinished`;
- register with `coat-runner-registry` through `RUNNER_REGISTRY_URL`;
- expose a Service only when other pods must call `/run-task`;
- set `runAsNonRoot`, `fsGroup`, `seccompProfile`, dropped capabilities, resource
  limits, and `HOME=/workspace` when running the toolbox image as UID 1000;
- use runner labels for auth locality, hardware, sandbox backend, tenant, and
  model-provider selection;
- run with least privilege and namespace NetworkPolicies.

## Network Policies

Runner and sandbox namespaces should default-deny ingress and egress, then add
small allow policies for the exact task profile. The example manifest splits
these policies instead of using one broad rule:

- `deny-by-default`: isolates all pods in `jattg-ephemeral`;
- `allow-dns`: permits DNS only for pods that need service discovery;
- `allow-runner-ingress-from-coat`: allows coordinator/control-plane pods to
  call exposed runner or temporary executor Services;
- `allow-control-plane-egress`: allows runner registration, memory lookup, and
  object-artifact writes;
- `allow-model-endpoint-egress`: allows model-provider pods to reach a labeled
  `jattg-models` namespace on vLLM/Ollama-style ports;
- `allow-restate-executor-egress`: lets temporary Restate service executors
  reach Restate admin/ingress and COAT service dependencies.

Use `jattg.dev/network-profile` labels to select one policy bundle per runner
purpose. Keep Codex or Claude Code device-auth runners node-local and avoid
open egress unless a goal's approval policy explicitly allows it.

## Restate Executors

Restate service handlers normally run as Deployments because Restate needs a
reachable endpoint. A Kubernetes Job is acceptable for a burst or migration-like
executor if:

- a Service selects the Job pod;
- the executor is registered with Restate after the pod is ready;
- `activeDeadlineSeconds` bounds the executor lifetime;
- workflows can tolerate the executor disappearing and retrying elsewhere.

Do not use a Job for the only production coordinator unless the operator accepts
that deadline-driven shutdown behavior.

## Helm

The chart supports disabled-by-default `.Values.ephemeralJobs`. Each entry can
set:

- `kind`: the `COAT_EPHEMERAL_KIND`;
- image, command, args, env/configEnv/secretEnv/fieldEnv;
- ports and optional Service generation;
- runtime class, node selector, tolerations, affinity, resource limits, volumes,
  pod/container security context, and injection mounts;
- optional per-Job NetworkPolicy with ingress and egress allowlists;
- `ttlSecondsAfterFinished`, `activeDeadlineSeconds`, and `backoffLimit`.

Example:

```yaml
ephemeralJobs:
  model-provider-burst:
    enabled: true
    kind: model-provider-runner
    image:
      repository: ghcr.io/josephjohncox/joseph-and-the-amazing-technicolor-task-graph/jattg-agent-toolbox
      tag: 0.0.2
    activeDeadlineSeconds: 7200
    ttlSecondsAfterFinished: 3600
    service: true
    podSecurityContext:
      runAsNonRoot: true
      runAsUser: 1000
      runAsGroup: 1000
      fsGroup: 1000
      seccompProfile:
        type: RuntimeDefault
    securityContext:
      allowPrivilegeEscalation: false
      capabilities:
        drop: ["ALL"]
    env:
      HOME: /workspace
      PORT: "9093"
      MODEL_PROVIDER_KIND: vllm
      MODEL_PROVIDER_ENDPOINT: http://vllm:8000/v1
      RUNNER_REGISTRY_URL: http://runner-registry:9085
      RUNNER_ENDPOINT: http://model-provider-burst:9093
    ports:
      - name: http
        containerPort: 9093
        servicePort: 9093
```

Render before applying:

```sh
helm template jattg infra/helm/jattg -f infra/helm/jattg/values-ephemeral-example.yaml
```
