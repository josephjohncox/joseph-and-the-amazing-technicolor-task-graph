# 070 Sandbox Tooling

## Objective

Create safe workspace lifecycle and deterministic Rust tool surfaces for workers.

## Implementation

- Create isolated workspace directories per task.
- Apply sandbox profiles for filesystem, network, and approval policy.
- Support optional strong sandbox backend selection for container, gVisor, Kata, Firecracker, Kubernetes Job, namespace jail, and provider-backed runners.
- Return sandbox attestations so validators and guardrail reviewers can distinguish enforced isolation from metadata-only local workspaces.
- Snapshot workspaces after task runs.
- Clean workspaces by goal/task ID.
- Expose repo status, local command planning/execution, test command, checkpoint, and artifact manifest tools through the tool registry.

## Implemented

- `coat-sandbox-runner` creates deterministic workspace IDs from goal/task IDs.
- `/workspaces` creates a per-task directory under `SANDBOX_WORKSPACE_ROOT`, writes `workspace-manifest.json`, and records a registry file for later snapshot/cleanup.
- `/snapshot` writes `snapshots/latest.json` when the workspace is known and is safe to call repeatedly.
- `/cleanup` removes the workspace and registry record only when the path is inside `SANDBOX_WORKSPACE_ROOT`; repeated cleanup returns `not_found`.
- Git and object-storage result refs are returned as contracts without mutating a source repository or uploading blobs.
- `coat-sandbox-runner` writes `artifacts/artifact-manifest.json` locally as an object-artifact validation contract; when object storage is configured it records planned artifact-manifest and snapshot-manifest `s3://` refs with `upload_performed_by_sandbox_runner=false`.
- `/snapshot` writes a local `object_upload` contract in `snapshots/latest.json` so validators can inspect the intended snapshot upload target without requiring live S3.
- `SandboxProfile.isolation` now captures backend, runtime class, seccomp/AppArmor profile, dropped capabilities, read-only rootfs intent, limits, egress policy ref, and snapshot strategy.
- `SandboxProfile.isolation` also carries ingress policy refs and network-policy labels so runners can attach Kubernetes, Cilium, Calico, cloud firewall, or provider sandbox guardrails without embedding policy syntax in task state.
- `coat-sandbox-runner` returns `SandboxAttestation`; local Compose defaults to `local_workspace` and does not pretend to enforce gVisor/Kata/Firecracker.
- `coat-sandbox-runner` exposes `/launch-plan` and writes `sandbox-launch-plan.json` so real executor adapters can consume the same durable plan without parsing free-form task prompts.
- `coat-sandbox-runner` exposes `/commands/plan` for approval-aware command routing; it returns `waiting_approval` until an approval ID is present when command approval is required.
- `coat-sandbox-runner` exposes `/commands/run` as an opt-in local executor when `SANDBOX_ENABLE_LOCAL_COMMAND_EXECUTION=true`; it runs only bare allowlisted binary names inside a known workspace, enforces task-local `ExecutionProfile.local_tools` binary/subcommand/denied-argument policy when supplied, and writes command evidence artifacts.
- `coat-sandbox-runner` exposes `/kubernetes/executor-jobs/provision`; plan-only mode projects `SandboxLaunchPlan` into ConfigMap and Job objects, while live modes use Rust `kube`/`k8s-openapi` server-side apply when `SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`.
- `coat-tool-registry` resolves `artifact_manifest` through `TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT` and returns workspace, launch-plan, snapshot, and artifact-manifest JSON when it can read the sandbox volume.
- `coat-tool-registry` routes MCP `test_command` calls to `/commands/plan` when `TOOL_REGISTRY_SANDBOX_RUNNER_URL` is configured; otherwise it reports sandbox delegation without execution.
- `coat-tool-registry` exposes MCP `local_command` for sandbox-runner planning or execution without executing local binaries inside the registry process.
- `ExecutionProfile.guardrails` can fork output and security guardrail reviews for completed work tasks.
- `coat-sandbox-runner` supports approval-gated live git worktree creation when `SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true`, the repo is under `SANDBOX_APPROVED_GIT_REPO_ROOTS`, and the request supplies `live_git_worktree.approval_id`.
- `jattg-agent-toolbox` provides a shared ephemeral Kubernetes Job image for bounded runner pods and temporary Restate service executors, while slim service images remain the default for always-on Deployments.
- `infra/k8s/examples/ephemeral-agent-runner-jobs.yaml` shows deadline-bound model-provider, Claude Code, and coordinator executor Jobs with Services, registry/Restate registration, injection mounts, resources, and NetworkPolicy.
- `coat deploy cluster executor-job render/apply` remains an operator fixture/escape hatch for materializing one bounded per-task Kubernetes Job from `SandboxLaunchPlan`.

## Tests

- Workspace creation returns a stable artifact reference.
- Snapshot and cleanup endpoints are idempotent.
- Tool registry lists known tools.
- Artifact manifest lookup reads real sandbox workspace files in Compose.
- Object-storage artifact manifest and snapshot upload refs are validated locally without contacting MinIO or S3.
- Dangerous commands require approval metadata.
- Local command execution is disabled by default, requires approval by default, rejects binaries outside `SANDBOX_ALLOWED_LOCAL_BINARIES`, and blocks task-local denied binaries, denied subcommands, and denied arguments before execution.
- Live git worktree creation remains metadata-only unless all operator and approval gates are present.
- MCP test command planning returns `waiting_approval` without an approval ID and `ready_for_executor` with one.
- Kubernetes executor Job provisioning preserves plan task IDs, runtime class, resources, security context, workspace volume, and network labels in backend-provisioned Job objects.

## Follow-Ups

- Coordinate Kubernetes executor proof and object upload promotion through `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
- Promote content-addressed snapshot archives from local manifests into object storage when the external object-store upload adapter is live.
- Connect `SandboxProfile.isolation.backend = kubernetes_job` to coordinator-approved capacity provisioning and write enforcement attestations after live Kubernetes Job completion.
- Add provider-backed sandbox adapters where managed sandbox APIs can return attestations.

## Acceptance

- Sandbox runner and tool registry have health checks.
- Compose and Kubernetes expose internal service names for both.
- Kubernetes examples include RuntimeClass and task-pod patterns for strong sandboxing.
- Kubernetes examples include reusable ephemeral runner Jobs and a toolbox image path for bounded burst capacity.
- Runner and sandbox examples use default-deny NetworkPolicies with profile-specific ingress and egress allowlists.
