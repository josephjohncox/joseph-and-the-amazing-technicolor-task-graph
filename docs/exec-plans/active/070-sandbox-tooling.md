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
- Expose repo status, test command, and artifact manifest tools through the tool registry.

## Implemented

- `coat-sandbox-runner` creates deterministic workspace IDs from goal/task IDs.
- `/workspaces` creates a per-task directory under `SANDBOX_WORKSPACE_ROOT`, writes `workspace-manifest.json`, and records a registry file for later snapshot/cleanup.
- `/snapshot` writes `snapshots/latest.json` when the workspace is known and is safe to call repeatedly.
- `/cleanup` removes the workspace and registry record only when the path is inside `SANDBOX_WORKSPACE_ROOT`; repeated cleanup returns `not_found`.
- Git and object-storage result refs are returned as contracts without mutating a source repository or uploading blobs.
- `SandboxProfile.isolation` now captures backend, runtime class, seccomp/AppArmor profile, dropped capabilities, read-only rootfs intent, limits, egress policy ref, and snapshot strategy.
- `coat-sandbox-runner` returns `SandboxAttestation`; local Compose defaults to `local_workspace` and does not pretend to enforce gVisor/Kata/Firecracker.
- `coat-sandbox-runner` exposes `/launch-plan` and writes `sandbox-launch-plan.json` so real executor adapters can consume the same durable plan without parsing free-form task prompts.
- `coat-sandbox-runner` exposes `/commands/plan` for approval-aware command routing; it returns `waiting_approval` until an approval ID is present and never executes the command itself.
- `coat-tool-registry` resolves `artifact_manifest` through `TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT` and returns workspace, launch-plan, snapshot, and artifact-manifest JSON when it can read the sandbox volume.
- `coat-tool-registry` routes MCP `test_command` calls to `/commands/plan` when `TOOL_REGISTRY_SANDBOX_RUNNER_URL` is configured; otherwise it reports sandbox delegation without execution.
- `ExecutionProfile.guardrails` can fork output and security guardrail reviews for completed work tasks.
- `coat-sandbox-runner` supports approval-gated live git worktree creation when `SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true`, the repo is under `SANDBOX_APPROVED_GIT_REPO_ROOTS`, and the request supplies `live_git_worktree.approval_id`.

## Tests

- Workspace creation returns a stable artifact reference.
- Snapshot and cleanup endpoints are idempotent.
- Tool registry lists known tools.
- Artifact manifest lookup reads real sandbox workspace files in Compose.
- Dangerous commands require approval metadata.
- Live git worktree creation remains metadata-only unless all operator and approval gates are present.
- MCP test command planning returns `waiting_approval` without an approval ID and `ready_for_executor` with one.

## Follow-Ups

- Promote content-addressed snapshot archives from local manifests into object storage when the object-store upload adapter is live.
- Add a Kubernetes Job runner that creates real per-task pods with `runtimeClassName`, resource limits, NetworkPolicies, and attestation evidence.
- Add provider-backed sandbox adapters where managed sandbox APIs can return attestations.

## Acceptance

- Sandbox runner and tool registry have health checks.
- Compose and Kubernetes expose internal service names for both.
- Kubernetes examples include RuntimeClass and task-pod patterns for strong sandboxing.
