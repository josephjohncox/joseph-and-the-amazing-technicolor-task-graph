# 030 Codex Worker

## Objective

Wrap Codex as a bounded coding worker behind the shared `AgentRunRequest -> AgentRunResult` contract.

## Implementation

- Keep `stub` mode as the default local path.
- Add Codex App Server mode for rich local thread/session control.
- Add Codex MCP mode for callable-tool workflows.
- Persist thread IDs and artifact manifests in worker diagnostics/artifacts.
- Map task sandbox profiles to runner filesystem, network, and approval settings.

## Implemented

- Stub mode returns structured actor, critic, unifier, and research-shaped results.
- Sidecar auto-registers with the runner registry and reports capabilities/model routes.
- `/verify` reports Codex CLI availability, declared `@openai/codex-sdk` dependency, App Server configuration, and optional MCP/App Server probes.
- MCP and App Server verification probes are gated by `CODEX_VERIFY_MCP=1` and `CODEX_VERIFY_APP_SERVER=1`.
- Result refs include git branch/worktree and object-storage manifest locations without uploading blobs.

## Tests

- Stub mode returns a valid `AgentRunResult`.
- MCP health-check mode fails clearly when `codex mcp-server` is unavailable.
- Live App Server tests are gated by `CODEX_APP_SERVER_URL`.

## Follow-Ups

- Coordinate the first live worker proof through `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
- Replace stub-only Codex execution with live App Server and MCP adapters once credentials, sandbox isolation, and package verification are available in CI.
- Capture live session/thread IDs, checkpoint refs, git refs, and artifact manifests in a fixture that can be replayed without re-running Codex.

## Acceptance

- Sidecar starts with `npm run dev`.
- `/healthz` and `/run-task` return JSON.
- Live modes never run unless explicit environment gates are set.
