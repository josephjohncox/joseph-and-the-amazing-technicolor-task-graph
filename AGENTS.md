# Joseph And The Amazing Technicolor Task Graph

This repo builds a durable agent control plane.

Agents do work.
The coordinator owns truth.
Restate owns time.
Rust owns policy, state, tools, and deployment.

## Mission

Build a durable task tree for long-running autonomous engineering work.
Do not build one giant infinite agent loop.

The system should keep working until a goal is complete, blocked, cancelled, or budget exhausted.

## Source Of Truth

- Product intent: `docs/product-specs/jattg-v1.md`
- Architecture: `ARCHITECTURE.md`
- Active implementation plans: `docs/exec-plans/active/`
- Completed plans: `docs/exec-plans/completed/`
- Operational runbooks: `docs/operations/`
- JSON schemas: `schemas/`
- Shared Rust contracts: `crates/domain/`

Update docs when behavior or public contracts change.

## Harness Rules

- Keep the harness separate from model execution.
- Keep durable state in the coordinator, not in worker prompts.
- Workers may request child tasks, but only the coordinator may create them.
- Every worker response must use the structured result contract.
- Every task must have a budget, sandbox profile, role, and done criteria.
- Every dangerous operation needs an explicit approval path.
- Every live agent integration must have a stub mode for local smoke tests.

## Architecture Rules

- Restate workflow: durable outer loop and task-tree state.
- Rust services: coordinator, validator, sandbox runner, tool registry, CLI.
- TypeScript sidecars: Codex runner and staff-engineer runner.
- Codex App Server is the preferred Codex integration.
- Codex MCP is the fallback callable-tool integration.
- `@ctxr/agent-staff-engineer` is a specialized worker, not the platform core.
- Compose and Kubernetes must run the same logical service boundaries.

## Subagent Routing

- Planner: decompose goals and propose the next frontier.
- Codex: make bounded code changes inside a sandbox.
- StaffEngineerClaude: issue-to-PR lifecycle, review loop, CI comment loop.
- Research: collect current facts and cite sources.
- Tester: add or run focused regression checks.
- Reviewer: inspect diffs for correctness, safety, and missing tests.
- Validator: decide whether task evidence satisfies done criteria.
- PatchMerger: combine artifacts only after validation passes.
- RustTool: deterministic internal tools exposed through the registry.

## Rust Workspace

- `crates/domain`: shared schemas, task-tree logic, budget and spawn policy.
- `crates/coordinator`: Restate workflow and durable service handlers.
- `crates/runner-registry`: distributed runner registration, heartbeat, and routing.
- `crates/notifier`: notification and human-feedback delivery surface.
- `crates/validator`: standalone validation service.
- `crates/sandbox-runner`: workspace lifecycle and snapshot service.
- `crates/tool-registry`: MCP-facing tool registry surface.
- `crates/cli`: `jattg` operator CLI.

## Sidecars

- `sidecars/codex-runner-ts`: Codex App Server or MCP adapter.
- `sidecars/staff-engineer-runner-ts`: `@ctxr/agent-staff-engineer` adapter.

Sidecars must return domain-compatible JSON and must support stub mode.

## Distributed Execution

- Every `TaskNode` has an `execution` profile.
- Runner selection uses role, capabilities, labels, locality, and optional runner ID.
- Model routing can target Codex, OpenAI, OpenAI-compatible endpoints, vLLM, Ollama, llama.cpp, Hugging Face, or local processes.
- Personas are task-local. Do not infer persona only from worker role.
- MCP context is distributed as server refs and secret refs, never raw tokens.
- Runners resolve MCP auth through env, Kubernetes Secret, Vault, cloud secret stores, workload identity, or OAuth delegation.
- Notifications are task-local and should be emitted for approval, feedback, blocked, failed, and completed events.

## Testing

- Run `cargo test --workspace` for Rust logic.
- Run `cargo run -p jattg-domain --bin generate-schemas -- schemas` after contract edits.
- Run `cargo check --workspace` before handing off.
- Validate Compose with `docker compose -f infra/compose/docker-compose.yml config`.
- Validate Kubernetes with `kubectl apply --dry-run=client -f infra/k8s/base/all.yaml` when `kubectl` is available.

## Deployment

- Local stack: `docker compose -f infra/compose/docker-compose.yml up --build`
- CLI local stack: `cargo run -p jattg-cli -- compose up`
- Kubernetes render: `cargo run -p jattg-cli -- k8s render --output infra/k8s/rendered.yaml`
- Restate ingress defaults to `http://localhost:8080`.
- Coordinator service listens on `:9080`.
- Runner registry listens on `:9085`.
- Notifier listens on `:9086`.

## Safety

- Never run live coding agents without an isolated workspace.
- Never use approval-policy `never` outside an isolated runner.
- Never let recursive child spawning bypass budget checks.
- Never merge or mark tracker work Done from an autonomous worker.
- Do not hide partial, blocked, or failed worker states.

## Documentation Style

- Keep root docs concise.
- Put detailed implementation steps in execution plans.
- Prefer decision-complete plans over vague roadmaps.
- Record assumptions and validation commands.
- Use diagrams when they clarify service boundaries.

## Current Build State

The first scaffold includes buildable contracts, service stubs, deployment manifests, and sidecar adapters.
Live Codex, OpenAI Agents SDK, and staff-engineer integrations are intentionally gated by environment and verification.
