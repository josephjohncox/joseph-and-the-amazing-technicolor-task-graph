# Joseph and the Amazing Technicolor Task Graph

This repo builds a durable agent control plane.

Use `coat` as the short slug for commands, packages, environment variables, service images, and deployment names.

Agents do work.
The coordinator owns truth.
Restate owns time.
Rust owns policy, state, tools, and deployment.

## Mission

Build a durable task tree for long-running autonomous engineering work.
Do not build one giant infinite agent loop.

The system should keep working until a goal is complete, blocked, cancelled, or budget exhausted.

## Source Of Truth

- Product intent: `docs/product-specs/coat-v1.md`
- Architecture: `ARCHITECTURE.md`
- Goal authoring guide: `docs/operations/goal-authoring.md`
- Distributed memory guide: `docs/design-docs/030-distributed-memory-knowledgebases.md`
- Result channels guide: `docs/design-docs/060-result-channels-git-object-storage.md`
- Protobuf and goal-store guide: `docs/design-docs/070-protobuf-goal-store-protocols.md`
- Events, webhooks, and schedules guide: `docs/design-docs/080-events-webhooks-schedules.md`
- Active implementation plans: `docs/exec-plans/active/`
- Completed plans: `docs/exec-plans/completed/`
- Operational runbooks: `docs/operations/`
- Protobuf contracts: `proto/coat/v1/`
- JSON schemas: `schemas/`
- Shared Rust contracts: `crates/domain/`
- Event API contract: `docs/api/event-gateway.asyncapi.yaml`
- Operational database migrations: `infra/db/migrations/`

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
- Protobuf contracts: cross-service and database-facing API surfaces.
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
- `crates/event-gateway`: webhook, calendar, scheduled-event, and triggered-goal ingress.
- `crates/goal-store`: queryable goal/task/event projection and local JSONL read model.
- `crates/runner-registry`: distributed runner registration, heartbeat, and routing.
- `crates/notifier`: notification and human-feedback delivery surface.
- `crates/memory-gateway`: local memory write, search, fork/join, and MCP memory surface.
- `crates/validator`: standalone validation service.
- `crates/sandbox-runner`: workspace lifecycle and snapshot service.
- `crates/tool-registry`: MCP-facing tool registry surface.
- `crates/cli`: `coat` operator CLI, packaged as `coat-cli`.

## Sidecars

- `sidecars/codex-runner-ts`: Codex App Server or MCP adapter.
- `sidecars/staff-engineer-runner-ts`: `@ctxr/agent-staff-engineer` adapter.

Sidecars must return domain-compatible JSON and must support stub mode.
Sidecars should self-register with the runner registry when `RUNNER_REGISTRY_URL` is set.

## Distributed Execution

- Every `TaskNode` has an `execution` profile.
- Every `TaskNode` also has a `purpose`: work, review, unification, or actor retry.
- Goals have `control_policy`, `research_policy`, `memory_policy`, and `approval_policy`; preserve them when editing contracts.
- Good goals are executable contracts: objective, evidence, constraints, memory context, research needs, execution profile, budgets, and approval risks.
- Non-trivial goals should include `authoring`, `plan.subgoals`, and stable `subgoal_id`s on known `initial_tasks`.
- Use `GoalProgress`, `TaskQuery`, and `TaskList` for progress and task distribution; do not ask workers to discover subgoals from prose.
- Initial tasks are coordinator-owned work seeds. Workers may request children, but subgoal creation and routing stay in durable state.
- Steering directives are the human control surface for pausing, resuming, injecting tasks, and requesting research.
- Runner selection uses role, capabilities, labels, locality, and optional runner ID.
- Model routing can target Codex, OpenAI, OpenAI-compatible endpoints, vLLM, Ollama, llama.cpp, Hugging Face, or local processes.
- Dispatch decisions should preserve ranked candidates and rejected-runner reasons for operator debugging.
- Sidecars should expose `/capabilities` without leaking MCP or provider secrets.
- Personas are task-local. Do not infer persona only from worker role.
- MCP context is distributed as server refs and secret refs, never raw tokens.
- Runners resolve MCP auth through env, Kubernetes Secret, Vault, cloud secret stores, 1Password, Bitwarden, Doppler, SOPS, workload identity, external brokers, or OAuth delegation.
- Device/browser auth for Codex or Claude Code is runner-local unless `AuthDistributionPolicy` explicitly allows brokered user auth or secret sync.
- Brokered user auth requires a human approval gate and short-lived leases; never place raw user tokens in task state, diagnostics, artifacts, or memory.
- Notifications are task-local and should be emitted for approval, feedback, blocked, failed, and completed events.
- Local notification threads are for operator visibility; Restate workflow state remains the source of truth.
- Goal satisfaction is gated by actor output, critic reviews, optional review unification, and a satisfaction score.
- Learning signals are reward-like validation/review scores for future actor/critic tuning; they are not permission to run unbounded retries.
- Research tasks must return sourced `ResearchOutput` plus an `InformationUsePlan`.
- Default durable semantic memory is Zep/Graphiti over MCP with Qdrant-backed embedded retrieval.
- Use `coat-memory-gateway` as the stable local interface before wiring live Graphiti/Zep or Qdrant calls.
- Use `memory_context` before substantial task work when a worker needs scoped durable context.
- Use `memory_repair` for adapter replay after Graphiti, Qdrant, or embedding credentials were unavailable.
- Distributed memory context is passed by reference; use scoped retrieval and provenance instead of prompt dumps.
- Code results should return `git_result` refs for task branches, worktrees, commits, and PRs.
- Large generated assets should return `object_artifacts` refs to S3-compatible storage; do not put large blobs in workflow state.
- Use one branch and one object prefix per task unless a unifier explicitly joins branches or promotes artifacts.
- Sandbox workspaces are rooted at `SANDBOX_WORKSPACE_ROOT`; snapshot and cleanup must be idempotent and must not remove paths outside that root.
- Postgres is the standard production goal read model. Restate remains authoritative; the goal store is a projection.
- Use the migration files for dashboard/audit database setup; do not infer database schema from ad hoc JSONL logs.
- Keep protobuf ID/status/artifact fields typed and put full Rust payloads behind JSON-schema envelopes.
- External events enter through `coat-event-gateway`; webhooks, cron, calendar checks, and event buses must not invoke workers directly.
- Webhook auth must use `WebhookAuthPolicy` and `SecretRef`; shared-secret, bearer, and HMAC-SHA256 are local gateway paths, while mTLS/OIDC should terminate in trusted ingress or secret middleware until implemented.
- Agent-proposed monitors or schedules require coordinator or human-approved activation.
- Recurring work should become events, triggered goals, or steering directives, not hidden sleeping loops inside agents.

## Testing

- Run `cargo test --workspace` for Rust logic.
- Run `cargo run -p coat-domain --bin generate-schemas -- schemas` after contract edits.
- Run `buf lint` after proto edits.
- Run `cargo check --workspace` before handing off.
- Validate Compose with `docker compose -f infra/compose/docker-compose.yml config`.
- Validate Kubernetes with `kubectl apply --dry-run=client -f infra/k8s/base/all.yaml` when `kubectl` is available.

## Deployment

- Local stack: `docker compose -f infra/compose/docker-compose.yml up --build`
- CLI local stack: `cargo run -p coat-cli -- compose up`
- Kubernetes render: `cargo run -p coat-cli -- k8s render --output infra/k8s/rendered.yaml`
- Restate ingress defaults to `http://localhost:8080`.
- Coordinator service listens on `:9080`.
- Runner registry listens on `:9085`.
- Notifier listens on `:9086`.
- Goal store listens on `:9088`.
- Event gateway listens on `:9089`.

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
