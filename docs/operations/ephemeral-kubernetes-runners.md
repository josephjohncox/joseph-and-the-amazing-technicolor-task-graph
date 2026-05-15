# Ephemeral Kubernetes Runners

Use ephemeral Kubernetes Jobs when a goal needs burst runner capacity, a
time-boxed local-model worker, a short-lived Claude Code/Codex runner, or a
temporary Restate service executor. Restate and the coordinator still own
durable state; Jobs only provide execution capacity.

Ephemeral capacity should be requested by the coordinator or executor framework
from approved templates, not hand-created by an operator for each task. A task
expresses this through `ExecutionProfile.capacity`, and the provisioner backend
is part of that policy:

```json
{
  "mode": "prefer_registered_then_ephemeral",
  "provisioner": {
    "backend": "kubernetes_controller",
    "controller_ref": "coat-kubernetes-provisioner",
    "namespace": "jattg-ephemeral",
    "service_account": "jattg-ephemeral-runner",
    "field_manager": "coat-kubernetes-provisioner",
    "allow_manual_manifest_escape_hatch": true
  },
  "template_refs": [
    {
      "name": "codex-burst",
      "kind": "runner_job",
      "required_capabilities": ["code", "git_worktree"],
      "config_ref": "jattg-ephemeral-runner-templates",
      "active_deadline_seconds": 7200,
      "ttl_seconds_after_finished": 1800
    }
  ],
  "request_timeout_seconds": 600,
  "max_pending_provisions": 1,
  "require_human_approval": true
}
```

The provisioner resolves the template, creates the bounded Job or temporary
executor through the Kubernetes control plane, waits for normal runner
registration or Restate service registration, and then dispatches through the
same registry path as persistent runners. `registered_runners_only` remains the
default.

## Capacity Decisions

Do not size runner pools from prompt prose. Size them from durable queue state.

The coordinator should group demand by execution profile:

- worker role and purpose: actor, reviewer, tester, research, unification,
  event processor, or SRE/data-engineering task;
- required capabilities and local tools;
- model route, sandbox backend, network profile, and locality;
- labels such as `pool`, `tenant`, `hardware`, `auth` locality, or an internal
  compatibility label such as `labels.lane`.

For each group, compute:

- queued runnable tasks;
- unmatched runnable tasks that failed dispatch;
- currently running tasks;
- blocked tasks that need a specific scarce capability;
- event backlog from webhooks, SQS, calendars, monitoring, IDE/LSP, PR/CI, or
  scheduled processors;
- pending provisions already requested.

The runner registry exposes `POST /capacity/plan` for a bounded recommendation.
It combines the demand above with runner heartbeats and
`CapacityScalingPolicy`. The policy controls min/max runners, slots per runner,
target backlog per runner, utilization target, event weighting, headroom,
cooldowns, and max scale steps. A recommendation is not permission to provision;
the coordinator still applies budget, approvals, template refs, and
`ExecutionProfile.capacity.mode`.

Operators can inspect the same recommendation without provisioning:

```sh
coat runner capacity-plan --file examples/runner-scaling-request.json
```

When the request file omits `policy`, `coat runner capacity-plan` fills it from
the resolved COAT profile: `config.runner_capacity.lane_policies[pool_key]`
first, then `config.runner_capacity.default_policy`. Use
`coat setup config --show` to inspect the active policy and
`examples/coat-user-config.json` for a user-level override template.

Use these defaults:

- local personal stack: `enabled=false` or `recommend_only`;
- trusted development cluster: `recommend_only` plus manual approval;
- production ephemeral runner pools: `provision_ephemeral`, small
  `max_scale_up_step`, finite `max_runners`, and human approval unless the pool
  is low-risk and fully sandboxed;
- event processors: scale from event backlog only for approved event-source
  routes, with small headroom and dead-letter queues.

Scale-down should normally mean "stop assigning new work and let Jobs expire by
TTL." Persistent runner Deployments can still use HPA/KEDA from normal
Kubernetes metrics, but the coordinator remains the source of truth for task
demand and provisioning approvals.

Use the standard Rust Kubernetes client stack for live control-plane operations:
`kube` plus `k8s-openapi`. The current sandbox-runner exposes
`POST /kubernetes/executor-jobs/provision`; plan-only mode returns the exact
ConfigMap and Job objects, while live mode uses Kubernetes server-side apply
when `SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`.

The first live proof should run against kind or k3d before any managed-cluster
automation is trusted. That proof must start from a coordinator-approved
capacity decision, not a hand-applied manifest: the coordinator evaluates the
task budget, approval policy, sandbox profile, `ExecutionProfile.capacity`, and
runner-registry capacity recommendation, then issues a provision request to the
sandbox-runner. The sandbox-runner is the only component in the proof that
materializes the executor Job. Operators may render the same objects for
inspection, but the accepted path is the API request plus Kubernetes watch and
result ingestion flow.

## Image Model

The default image is `jattg-agent-toolbox`, built from the `agent-toolbox` target
in `infra/containers/rust-service.Dockerfile`.

It includes:

- all Rust service binaries and the `coat` CLI;
- built TypeScript runner sidecars for Codex, Claude Code, model-provider, and staff-engineer;
- common operator tools: `bash`, `curl`, `git`, `jq`, `ripgrep`, `openssh-client`, Python, build tools, Node, and npm;
- `/usr/local/bin/jattg-ephemeral-entrypoint`, which dispatches by `COAT_EPHEMERAL_KIND`.

The toolbox image can satisfy `local_commands`, `git_cli`, `build_tooling`, and
`package_manager_cli` for tasks that declare `ExecutionProfile.local_tools`.
Do not advertise `docker_cli`, `helm_cli`, or `kubernetes_cli` unless those
binaries and their credentials are injected or installed in the runner image and
the runner has the matching sandbox and network policy.

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

## Template Library

`infra/k8s/examples/ephemeral-agent-runner-jobs.yaml` is an operator fixture and
escape hatch. It mirrors the templates that should normally live in
`.Values.ephemeralRunnerTemplates` and be consumed by the backend provisioner.
It includes examples for:

- a model-provider runner Job that can point at vLLM, Ollama, Bedrock, or an
  OpenAI-compatible gateway;
- a Claude Code runner Job with node-local or secret-backed auth;
- a temporary coordinator Restate service executor with a Service and
  registration Job.

Render the reusable raw manifest set from the CLI only when you want an operator
example, CI fixture, or emergency manual apply:

```sh
coat deploy cluster ephemeral-jobs render \
  --output infra/k8s/rendered-ephemeral-agent-runner-jobs.yaml
```

Provisioned Runner Jobs should:

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

## Per-Task Executor Jobs

`SandboxLaunchPlan` is the durable handoff from the sandbox runner to a concrete
executor. The backend provisioner path is:

1. the coordinator marks a runnable task eligible for
   `SandboxBackend::KubernetesJob` only after budget, approval, sandbox, local
   tool, capacity, and template-ref checks pass;
2. the coordinator or executor framework sends the sandbox-runner a provision
   request containing the durable task ID, approved `SandboxLaunchPlan`, target
   namespace, field manager, TTL, deadline, service account, runtime class, and
   expected result locations;
3. sandbox-runner validates the launch plan, resolves the approved template, and
   calls Kubernetes with the standard Rust `kube` client;
4. the provisioner applies a ConfigMap containing `sandbox-launch-plan.json` and
   a bounded Job through server-side apply;
5. sandbox-runner watches the Job and selected Pod until completion, timeout,
   deletion, or a classified Kubernetes failure such as image-pull,
   unschedulable, runtime-class, admission, or deadline failure;
6. the executor reads `sandbox-launch-plan.json`, runs only the bounded command
   contract, writes command, artifact, checkpoint, git, object-store, and
   structured-result manifests, and emits sandbox attestation evidence;
7. sandbox-runner collects pod logs, final Job/Pod manifests, result manifests,
   exit status, timing, cleanup status, and attestation evidence, then returns a
   normalized result record to the coordinator;
8. the coordinator validates the returned evidence, updates task state, and
   projects the result into goal-store.

Plan a bounded Job without contacting the cluster:

```sh
curl -sS -X POST http://localhost:9083/kubernetes/executor-jobs/provision \
  -H 'content-type: application/json' \
  --data @examples/kubernetes-executor-job-provision.json
```

When `SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`, set
`"mode": "server_dry_run"` or `"mode": "apply"` in the provision request to use
the live Kubernetes API. The endpoint applies a ConfigMap containing the
`SandboxLaunchPlan` and a bounded Job using server-side apply and the request's
`field_manager`.

Use `"mode": "server_dry_run"` for the first kind/k3d proof stage: it validates
admission, defaulting, RBAC, namespace policy, service account permissions, and
runtime class references without launching work. Use `"mode": "apply"` only
after that dry-run returns the expected ConfigMap and Job and the coordinator
has recorded the approval that allowed this capacity request.

Live `server_dry_run` and `apply` requests must include coordinator evidence
annotations before sandbox-runner contacts the cluster:

- `jattg.dev/capacity-decision-ref`: the durable capacity decision or approval
  record that authorized this executor Job;
- `jattg.dev/template-ref`: the approved template or template-library entry used
  to materialize the Job;
- `jattg.dev/result-ingestion-ref`: the coordinator or goal-store destination
  that will ingest the structured executor result.

The sandbox-runner also writes `provisioner-evidence.json` beside
`sandbox-launch-plan.json` in the launch-plan ConfigMap, mounts it at
`/coat/provisioner-evidence.json`, and injects `COAT_PROVISION_REQUEST_ID`,
`COAT_STRUCTURED_RESULT_PATH`, `COAT_SANDBOX_ATTESTATION_PATH`,
`COAT_PROVISIONER_EVIDENCE_PATH`, and Kubernetes watch selector environment
variables into the executor container.

The CLI renderer remains useful for inspection and dry-run fixtures:

```sh
coat deploy cluster executor-job render \
  --launch-plan examples/sandbox-launch-plan-kubernetes-job.json \
  --output /tmp/jattg-executor-job.json
coat deploy cluster executor-job apply \
  --launch-plan examples/sandbox-launch-plan-kubernetes-job.json \
  --output /tmp/jattg-executor-job.json \
  --dry-run=client
```

The rendered manifest includes a ConfigMap copy of the launch plan, task labels,
runtime class, security context, resource limits, workspace volume, plan
environment, and network-policy labels. The executor writes artifacts back
through the task workspace or object-store refs; the coordinator then validates
the result and records the attestation.

The Job watch result should preserve enough evidence for later review:

- provision request ID, goal ID, task ID, sandbox profile, template ref, and
  capacity decision ref;
- applied ConfigMap and Job object UIDs plus final resource versions;
- selected Pod UID, node name when available, phase transitions, container
  statuses, restart count, exit code, signal, reason, and message;
- first and final timestamp for pending, running, succeeded, failed, timeout,
  cleanup, and TTL observation;
- bounded pod logs with truncation metadata and object-artifact refs for full
  logs when needed;
- command, artifact, checkpoint, git, object-store, and structured-result
  manifests written by the executor;
- sandbox attestation evidence describing namespace, service account, image
  digest when available, runtime class, security context, network profile,
  mounted launch-plan ConfigMap, and cleanup outcome.

Do not count a Kubernetes Job as satisfying the task only because the Pod
reached `Succeeded`. The coordinator should require the structured result
manifest and attestation evidence, then let validator or review tasks decide
whether the objective-level evidence is sufficient.

## Helm

The chart exposes `.Values.ephemeralRunnerTemplates` as the standard capacity
template library for coordinator/executor provisioning. Helm renders these
templates into the `jattg-ephemeral-runner-templates` ConfigMap. A provisioner
should read that ConfigMap, select a template from
`ExecutionProfile.capacity.template_refs`, and materialize a bounded Job only
after budget and approval checks pass.

`.Values.ephemeralJobs` remains as a disabled-by-default manual escape hatch, not
the normal autoscaling or task-execution path.
Each manual entry can
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
      tag: 0.0.3
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
coat deploy chart template \
  --values infra/helm/jattg/values-ephemeral-example.yaml \
  --output /tmp/jattg-ephemeral.yaml
```
