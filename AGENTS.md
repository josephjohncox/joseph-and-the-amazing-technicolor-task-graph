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
- Documentation map: `docs/README.md`
- Goal authoring guide: `docs/operations/goal-authoring.md`
- Runner context initialization guide: `docs/operations/runner-context-initialization.md`
- Distributed memory guide: `docs/design-docs/030-distributed-memory-knowledgebases.md`
- Multi-user OIDC MCP guide: `docs/design-docs/130-multi-user-oidc-mcp.md`
- Result channels guide: `docs/design-docs/060-result-channels-git-object-storage.md`
- Protobuf and goal-store guide: `docs/design-docs/070-protobuf-goal-store-protocols.md`
- Events, webhooks, and schedules guide: `docs/design-docs/080-events-webhooks-schedules.md`
- Strong sandbox and guardrails guide: `docs/design-docs/100-strong-sandboxing-guardrails.md`
- Control gateway and SPA guide: `docs/design-docs/110-control-gateway-spa.md`
- Durable planning mode guide: `docs/design-docs/120-durable-planning-mode.md`
- Model and runner cluster guide: `docs/operations/model-runner-clusters.md`
- Restate Cloud runbook: `docs/operations/restate-cloud.md`
- Active implementation plans: `docs/exec-plans/active/`
- Completed plans: `docs/exec-plans/completed/`
- Operational runbooks: `docs/operations/`
- Release runbook: `docs/operations/releases.md`
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
- Any prompt, skill, MCP tool, runner context, or worker output that says
  "subagent" means a COAT durable child task unless explicitly stated otherwise.
- Codex, Claude Code, Agents SDK, MCP clients, and local-model runners must not
  spawn native in-process subagents. They return `ChildTaskRequest` values in
  `AgentRunResult.child_requests`; the coordinator queues and routes them.
- Every worker response must use the structured result contract.
- Every task must have a budget, sandbox profile, role, and done criteria.
- Every dangerous operation needs an explicit approval path.
- Strong sandboxing is opt-in by profile, but runners must never claim gVisor, Kata, Firecracker, or provider sandbox enforcement unless they can return an attestation.
- Executor output is untrusted data until validated and, when enabled, reviewed by output and security guardrail tasks.
- Every live agent integration must have a stub mode for local smoke tests.

## Architecture Rules

- Restate workflow: durable outer loop and task-tree state.
- Restate Cloud is supported for personal durable use and corporate managed deployment; configure service identity verification before exposing coordinator endpoints.
- Protobuf contracts: cross-service and database-facing API surfaces.
- Rust services: coordinator, validator, sandbox runner, tool registry, CLI.
- TypeScript sidecars: Codex runner and staff-engineer runner.
- Codex App Server is the preferred Codex integration.
- Codex MCP is the fallback callable-tool integration.
- `@ctxr/agent-staff-engineer` is a specialized worker, not the platform core.
- Compose and Kubernetes must run the same logical service boundaries.
- The TypeScript control gateway and SPA are optional operator surfaces; they must use backend APIs and must not own durable workflow state.

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
- `ui/control-plane-web`: optional TypeScript control gateway, SPA, and MCP dashboard surface.

Sidecars must return domain-compatible JSON and must support stub mode.
Sidecars should self-register with the runner registry when `RUNNER_REGISTRY_URL` is set.

## Distributed Execution

- Every `TaskNode` has an `execution` profile.
- Every `TaskNode` also has a `purpose`: work, review, unification, or actor retry.
- Every `TaskNode` may carry a `color` from `GoalSpec.color_policy`, subgoal metadata, or explicit child-task metadata; use stable color keys as semantic graph labels, not one-off UI decoration.
- Goals have `control_policy`, `research_policy`, `memory_policy`, and `approval_policy`; preserve them when editing contracts.
- Good goals are executable contracts: objective, evidence, constraints, memory context, research needs, execution profile, budgets, and approval risks.
- Use durable plans for chat-style planning before execution; revise and compile plans into `GoalSpec` instead of treating planning prose as worker-owned state.
- Non-trivial goals should include `authoring`, `plan.subgoals`, and stable `subgoal_id`s on known `initial_tasks`.
- Use `GoalProgress`, `TaskQuery`, and `TaskList` for progress and task distribution; do not ask workers to discover subgoals from prose.
- Initial tasks are coordinator-owned work seeds. Workers may request children, but subgoal creation and routing stay in durable state.
- Steering directives are the human control surface for pausing, resuming, injecting tasks, and requesting research.
- Runner selection uses role, capabilities, labels, locality, and optional runner ID.
- Model routing can target Codex, OpenAI, OpenAI-compatible endpoints, vLLM, Ollama, llama.cpp, Hugging Face, or local processes.
- Dispatch decisions should preserve ranked candidates and rejected-runner reasons for operator debugging.
- Sidecars should expose `/capabilities` without leaking MCP or provider secrets.
- Sandbox-capable runners should advertise backend capabilities and labels such as `sandbox.backend`, `sandbox.runtime_class`, and `network.egress`.
- Local workspace sandboxing is for trusted development only; production untrusted execution should use container hardening, gVisor, Kata, Firecracker, Kubernetes Jobs, or provider-backed sandboxes.
- Personas are task-local. Do not infer persona only from worker role.
- `ExecutionProfile.subagents` is the runner-context source of truth for
  subagent behavior. Default mode is `coordinator_durable_tasks`; default native
  subagent spawning is `disabled`.
- MCP context is distributed as server refs and secret refs, never raw tokens.
- Default access mode is `single_user`; multi-user OIDC is an opt-in extension through `McpContextRef.access_mode=multi_user_oidc`.
- Runners resolve MCP auth through env, Kubernetes Secret, Vault, cloud secret stores, 1Password, Bitwarden, Doppler, SOPS, workload identity, external brokers, or OAuth delegation.
- User-delegated MCP auth must use `UserPrincipalRef`, `OidcDelegationPolicy`, `McpAuthRef::OidcDelegation`, short-lived broker leases, `oidc_user_delegation` runner capability, and tenant/user labels.
- Device/browser auth for Codex or Claude Code is runner-local unless `AuthDistributionPolicy` explicitly allows brokered user auth or secret sync.
- Brokered user auth requires a human approval gate and short-lived leases; never place raw user tokens in task state, diagnostics, artifacts, or memory.
- Notifications are task-local and should be emitted for approval, feedback, blocked, failed, and completed events.
- Local notification threads are for operator visibility; Restate workflow state remains the source of truth.
- Web UI edits are steering, approval, goal, event, or memory commands against backend APIs; never mutate projections as if they were source-of-truth state.
- Agent progress views should read projected `TaskRecord` rows and `payload_json.prompt` so operators can inspect current prompts, task contracts, state, and evidence.
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
- Workers and subagents should return `checkpoints` for git commits, git branches, workspace snapshots, object archives, or metadata milestones so operators can inspect task history.
- Checkpoint refs are history pointers. Keep checkpoint payloads small and store full diffs, snapshots, or large bundles in git, workspace snapshot storage, or S3-compatible object storage.
- Use one branch and one object prefix per task unless a unifier explicitly joins branches or promotes artifacts.
- Sandbox workspaces are rooted at `SANDBOX_WORKSPACE_ROOT`; snapshot and cleanup must be idempotent and must not remove paths outside that root.
- Sandbox launch plans are durable contracts; real executors consume `sandbox-launch-plan.json` and return attestation/evidence instead of inferring runtime setup from prompts.
- Strict executor tasks should require `ExecutionProfile.guardrails`, artifact manifests, sandbox attestations, and bounded output/security review tasks before goal satisfaction.
- Model-serving pools and executor pools should be separate when possible; GB10/DGX Spark, Mac mini, GPU, and CPU nodes should register their real model and sandbox capabilities instead of relying on prompt convention.
- Postgres is the standard production goal read model. Restate remains authoritative; the goal store is a projection.
- Use the migration files for dashboard/audit database setup; do not infer database schema from ad hoc JSONL logs.
- Keep protobuf ID/status/artifact fields typed and put full Rust payloads behind JSON-schema envelopes.
- External events enter through `coat-event-gateway`; generic events, webhooks, cron, calendar checks, and event buses must not invoke workers directly.
- Use generic JSON or CloudEvents-compatible event sources for CI, git, issue tracker, chat, monitoring, database-change, memory, runner, and agent-topology events before adding provider-specific adapters.
- Webhook auth must use `WebhookAuthPolicy` and `SecretRef`; shared-secret, bearer, and HMAC-SHA256 are local gateway paths, while mTLS/OIDC should terminate in trusted ingress or secret middleware until implemented.
- Agent-proposed monitors or schedules require coordinator or human-approved activation.
- Recurring work should become events, triggered goals, or steering directives, not hidden sleeping loops inside agents.

## Testing

- Run `cargo test --workspace` for Rust logic.
- Run `make schemas` after contract edits.
- Run `buf lint` after proto edits.
- Run `cargo check --workspace` before handing off.
- Validate Compose with `docker compose -f infra/compose/docker-compose.yml config`.
- Validate Kubernetes with `kubectl apply --dry-run=client -f infra/k8s/base/all.yaml` when `kubectl` is available.

## Deployment

- Local stack: `docker compose -f infra/compose/docker-compose.yml up --build`
- Personal Restate Cloud stack: `docker compose --env-file infra/compose/restate-cloud.env -f infra/compose/docker-compose.yml -f infra/compose/docker-compose.restate-cloud.yml --profile restate-cloud up --build`
- CLI local stack: `coat compose up`
- CLI Restate Cloud stack: `coat compose up --restate-cloud`
- Restate Cloud registration: `coat restate register-cloud --tunnel-name coat-personal --service-url http://coordinator:9080`
- Kubernetes render: `coat k8s render --output infra/k8s/rendered.yaml`
- Restate ingress defaults to `http://localhost:8080`.
- Coordinator service listens on `:9080`.
- Runner registry listens on `:9085`.
- Notifier listens on `:9086`.
- Goal store listens on `:9088`.
- Event gateway listens on `:9089`.
- Control web listens on `:9090`.

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
- Every active execution plan must include `## Follow-Ups`; preserve unresolved follow-up work across turns until it is completed, explicitly superseded, or moved to another plan.
- Use `coat follow-ups` to inspect continuation items before choosing the next plan to advance.
- Record assumptions and validation commands.
- Use diagrams when they clarify service boundaries.

## Current Build State

The first scaffold includes buildable contracts, service stubs, deployment manifests, and sidecar adapters.
Live Codex, OpenAI Agents SDK, and staff-engineer integrations are intentionally gated by environment and verification.
