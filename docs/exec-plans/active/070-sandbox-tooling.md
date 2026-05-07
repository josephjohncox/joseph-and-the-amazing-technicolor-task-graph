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
- `coat-tool-registry` resolves `artifact_manifest` through `TOOL_REGISTRY_SANDBOX_WORKSPACE_ROOT` and returns workspace, launch-plan, snapshot, and artifact-manifest JSON when it can read the sandbox volume.
- `ExecutionProfile.guardrails` can fork output and security guardrail reviews for completed work tasks.

## Tests

- Workspace creation returns a stable artifact reference.
- Snapshot and cleanup endpoints are idempotent.
- Tool registry lists known tools.
- Artifact manifest lookup reads real sandbox workspace files in Compose.
- Dangerous commands require approval metadata.

## Follow-Ups

- Add an approval-gated live git worktree creation mode that runs only for explicitly approved local repositories.
- Promote content-addressed snapshot archives from local manifests into object storage when the object-store upload adapter is live.
- Route approved test commands through sandbox runner instead of the MCP tool registry.
- Add a Kubernetes Job runner that creates real per-task pods with `runtimeClassName`, resource limits, NetworkPolicies, and attestation evidence.
- Add provider-backed sandbox adapters where managed sandbox APIs can return attestations.

## Acceptance

- Sandbox runner and tool registry have health checks.
- Compose and Kubernetes expose internal service names for both.
- Kubernetes examples include RuntimeClass and task-pod patterns for strong sandboxing.
