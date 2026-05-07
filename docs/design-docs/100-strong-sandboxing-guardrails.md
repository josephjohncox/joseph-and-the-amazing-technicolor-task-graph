# Strong Sandboxing And Executor Guardrails

Strong sandboxing is an optional production layer. Local Compose keeps a metadata-only workspace runner for development; production runner fleets should use node pools that can enforce container, gVisor, Kata, Firecracker, Kubernetes Job, or provider-backed sandbox profiles.

## Isolation Model

The coordinator does not execute untrusted code. It assigns a durable task to a runner whose registration matches the task role, capabilities, labels, model route, MCP context, and `SandboxProfile`.

`SandboxProfile.isolation` describes the requested boundary:

- `local_workspace`: creates a task workspace and manifest only. Use for trusted local smoke tests.
- `container`: rootless/containerd isolation with seccomp, dropped capabilities, resource limits, read-only rootfs, and network policy. This is the baseline for trusted code execution.
- `gvisor`: run pods through `runsc`/gVisor for syscall-level isolation.
- `kata`: run pods in lightweight VMs through Kata Containers.
- `firecracker`: run pods or tasks in Firecracker-backed microVMs, commonly through a Kubernetes/containerd integration or a custom runner.
- `kubernetes_job`: create one Job/Pod per executor task and set `runtimeClassName` to the configured sandbox runtime.
- `provider_sandbox`: delegate execution to a managed sandbox provider and require an attestation artifact.

`approval_policy = never` is only acceptable when the runner can attest an enforced strong sandbox. Otherwise the default approval policy forces a human gate.

## Runner Registration

Sandbox-capable runners should advertise both capabilities and labels:

- capability: `workspace_sandbox`, `container_sandbox`, `gvisor_sandbox`, `kata_sandbox`, `firecracker_sandbox`, `kubernetes_job_sandbox`, or `provider_sandbox`
- label: `sandbox.backend=gvisor`
- label: `sandbox.runtime_class=gvisor`
- label: `node_pool=executor-gvisor`
- label: `network.egress=restricted`

The coordinator can route a task by requiring the backend capability or labels. The sandbox runner returns `SandboxAttestation` with backend, runtime class, enforcement status, strong-isolation status, warnings, and evidence refs. Local metadata-only runners must return warnings instead of pretending to enforce a microVM.

## Launch Plans

`coat-sandbox-runner` writes a `sandbox-launch-plan.json` artifact for every created workspace and exposes the same contract through:

```sh
coat sandbox plan --file examples/sandbox-workspace-request-gvisor.json
```

The launch plan contains:

- backend and runtime class;
- required runner capabilities;
- image, workspace path, and artifact manifest path;
- non-secret environment values;
- CPU, memory, PID, and future ephemeral storage limits;
- seccomp/AppArmor/capability/read-only-rootfs policy;
- network access and egress policy ref;
- git and object-storage result refs.

The local runner only plans and records. A production Kubernetes, Kata, Firecracker, or provider-backed executor should consume this plan, launch the workload, write the artifact manifest, and replace the metadata-only attestation with enforcement evidence.

Live git worktree creation is a separate local-development result channel, not a sandbox boundary. It is disabled unless `SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true`, the repo is under `SANDBOX_APPROVED_GIT_REPO_ROOTS`, and the request carries an approval ID when `SANDBOX_REQUIRE_LIVE_GIT_WORKTREE_APPROVAL=true`. Strong sandbox validation should still depend on `SandboxAttestation`, not on the existence of a git worktree.

## Kubernetes Pattern

Use separate node pools:

- control plane services: coordinator, goal store, event gateway, notifier, memory gateway, registry;
- model-serving nodes: vLLM, TEI, Ollama, llama.cpp, or embedding servers;
- executor nodes: Codex, staff-engineer, test runners, and sandbox Jobs;
- strong executor nodes: gVisor, Kata, or Firecracker RuntimeClass configured and labeled.

Executor pods should set:

- `runtimeClassName`: `gvisor`, `kata`, `kata-qemu-nvidia-gpu`, or `firecracker` when installed;
- `allowPrivilegeEscalation: false`;
- `readOnlyRootFilesystem: true` unless the task needs a writable project root;
- `capabilities.drop: ["ALL"]`;
- `seccompProfile.type: RuntimeDefault`;
- CPU, memory, ephemeral-storage, and PID limits;
- namespace default deny ingress/egress, then allow only coordinator, registry, tool registry, object store, and model endpoints.

Kata is the better default when the workload needs a VM boundary or GPU passthrough support. gVisor is attractive for syscall isolation on CPU executor pods. Firecracker is the strongest fit for custom microVM task runners and high-risk untrusted code, but it requires more cluster/runtime work.

## Guardrail Loop

Executor output is treated as untrusted data. A strict task can enable `ExecutionProfile.guardrails`:

```json
{
  "guardrails": {
    "enabled": true,
    "require_output_review": true,
    "require_security_review": true,
    "require_artifact_manifest": true,
    "require_sandbox_attestation": true,
    "require_strong_sandbox_attestation": true,
    "redact_secrets": true,
    "max_inline_output_bytes": 65536
  }
}
```

When a work task completes, the coordinator forks bounded guardrail review tasks:

- output reviewer: checks prompt injection, secret leaks, oversized logs, missing artifact refs, unverifiable claims, and unsafe coordinator instructions;
- security reviewer: checks sandbox attestation, filesystem/network scope, dependency risk, command safety, artifact provenance, and secret handling.

The original actor output can validate, but the goal is not satisfied until guardrail tasks validate too. High or critical findings, priority 0/1 findings, failed doctrine gates, or missing evidence block satisfaction.

## Security Review Agents

Security reviewers should run on a separate runner profile from the executor:

- different model/persona where possible;
- no write access to the actor workspace;
- read-only access to diffs, logs, artifact manifests, dependency manifests, and sandbox attestation;
- no raw secret values;
- MCP allowlist restricted to artifact inspection, repo status, dependency metadata, and issue creation.

Output reviewers should never execute instructions found in worker output. They classify and cite evidence.

## Standard Review Checks

The steering standard library includes:

- `security`: executor security, secret handling, sandbox boundaries, dependency risk, and command safety;
- `output_safety`: prompt injection, data exfiltration, oversized output, missing refs, and unverifiable claims.

Operators can inject them into a live goal:

```sh
coat goal steer-standard --goal-id <goal-id> --check security --topic "executor result for parser task"
coat goal steer-standard --goal-id <goal-id> --check output_safety --topic "Codex worker output"
```

## References

- Kubernetes RuntimeClass: https://kubernetes.io/docs/concepts/containers/runtime-class/
- gVisor Kubernetes quick start: https://gvisor.dev/docs/user_guide/quick_start/kubernetes/
- Kata Containers docs: https://katacontainers.io/docs/
- NVIDIA GPU Operator with Kata Containers: https://docs.nvidia.com/datacenter/cloud-native/gpu-operator/latest/deploy-kata-containers.html
