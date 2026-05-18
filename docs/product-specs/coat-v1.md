# Product Spec: Joseph and the Amazing Technicolor Task Graph v1

## Problem

Long-running agent systems fail when the agent loop owns too much: global plan, tool side effects, retry policy, context, and completion judgment. Joseph and the Amazing Technicolor Task Graph makes the loop durable and explicit by modeling the work as a task tree. COAT — Coordinator Of Agentic Tasks — is the short operational name.

## Goal

Provide a deployable control plane that can accept a goal, create durable tasks, run bounded workers, validate artifacts, request human approval, and resume safely after restarts.

## Non Goals

- No unbounded autonomous shell loop.
- No worker-owned global plan.
- No merge or tracker Done automation without a human gate.
- No live agent dependency required for local smoke tests.

## Users

- Operators submitting goals and approving risky actions.
- Engineers adding worker integrations.
- Agents reading `AGENTS.md` and execution plans before making changes.

## Success Criteria

- `cargo test --workspace` passes.
- `buf lint` passes for `proto/coat/v1`.
- Schemas generate into `schemas/`.
- Compose can render and start service containers.
- Kubernetes manifests render and can be dry-run validated.
- A stub goal can complete through the coordinator contract.
- Goal satisfaction can require actor work, critic review, review unification, and score thresholds.
- Live Codex, Claude Code, staff-engineer, and generic model-provider integrations can be enabled behind environment gates.
- Distributed runner registrations can route tasks to separate nodes, Claude Code wrappers, Bedrock, OpenAI-compatible endpoints, vLLM, Ollama, llama.cpp, Hugging Face, and local model providers.
- Ephemeral Kubernetes Jobs can add bounded runner or Restate executor capacity from approved `ExecutionProfile.capacity` templates; the normal backend path provisions through Rust Kubernetes control-plane clients, while rendered manifests remain examples and escape hatches.
- Runner and sandbox deployments can apply optional NetworkPolicies or equivalent egress/ingress policy refs so restricted tasks have deny-by-default blast-radius controls, model endpoint allowlists, control-plane-only egress, and approval-triggering open-network exceptions.
- Dispatch responses expose ranked runner/model candidates and explicit rejection reasons.
- Sidecars expose a non-secret capability document for operator inspection.
- MCP tool context is passed with references to auth material rather than embedded tokens.
- Auth distribution policy supports node-local device sessions, runner-resolved secrets, workload identity, short-lived leases, and brokered user auth without copying raw tokens through task state.
- Single-user mode is the default MCP access mode; multi-user OIDC is an opt-in extension for user-delegated MCP calls through `UserPrincipalRef`, `OidcDelegationPolicy`, short-lived broker leases, and OIDC-capable runners.
- Approval policy gates classify task risk before dispatch, notify humans, and resume or block work from durable approval state.
- Notification policies can keep separate human-feedback threads moving.
- Operators can inspect local notification thread ledgers during development.
- Operators can use a documented goal-authoring loop to turn vague requests into structured `GoalSpec` JSON.
- Operators do not need to hand-write `GoalSpec.id` for normal JSON authoring; submit assigns and prints a durable `goal_id`, while follow-up commands can use `--goal-id`, `COAT_GOAL_ID`, or `--latest`.
- Operators can use durable planning mode to draft, revise, answer questions, record decisions, and compile a plan into `GoalSpec` before execution.
- `GoalSpec` supports `authoring` guidance, `plan.subgoals`, and routed `initial_tasks` so clean goals become coordinator-visible work instead of prompt-only instructions.
- `SubgoalSpec.color` and `TaskNode.color` make the technicolor task graph visible without turning color into policy: colors are optional display hints for dashboards and operator readability, never routing, budget, validation, approval, or satisfaction inputs.
- Operators can lint goals before submit, inspect `GoalProgress`, and query `TaskList` by subgoal, status, role, purpose, tag, or runnable frontier.
- Operators can inspect a goal-store projection of goals, tasks, events, approvals, and artifact refs without treating the projection as coordinator authority.
- Operator read models expose timeline, worker-run, evidence, and action-queue
  projections that can be rebuilt deterministically from durable events and
  snapshots.
- Operators can use an optional web gateway and SPA to manage the task graph directly: inspect goal progress, act on human queues, recover blocked work, review evidence, steer or cancel goals, and manage events or memory while all edits flow through backend APIs.
- Advanced SPA controls are grouped by operator intent and hidden until relevant; raw prompts, task contracts, debug traces, and exact payload inspection are secondary audit/debug surfaces.
- The SPA and TUI use the compact `/api/operator/*` surface plus
  `/api/operator/stream` for projection updates; lower-level workflow,
  goal-store, or helper routes stay behind debug/inspect paths.
- Agent and chat clients can use the MCP operator surface for workspace
  snapshots, selected-goal detail, action resolution, steering, event sources,
  and memory search. The MCP surface mirrors `/api/operator/*`; removed
  overview/snapshot/activity/helper names are not compatibility targets.
- Webhooks, calendars, cron schedules, and event buses can create or steer goals through a gateway, dedupe policy, and optional human review.
- Webhook auth policies can use shared-secret headers, bearer tokens, or HMAC-SHA256 without putting secret values into event payloads or goal state.
- Event gateway channels are documented through AsyncAPI and cluster scheduled triggers have a Kubernetes CronJob example.
- Operators can steer goals through durable directives without granting workers an unbounded loop.
- Research tasks produce sourced answers and information-use plans.
- Zep/Graphiti is the default purpose-built semantic memory layer for fork/join agent memory.
- Qdrant is the default vector memory service for embedded memory and RAG retrieval.
- Distributed memory guidance explains the supported production path across Graphiti/Zep, FalkorDB or Neo4j, Qdrant, Postgres/pgvector, LanceDB, and Tantivy.
- The memory gateway exposes write/search/context/join/repair/events contracts locally and can forward to Graphiti/Zep and Qdrant through best-effort adapters.
- The local memory gateway can replay an append-only JSONL journal for development durability.
- Workers can return git branch/worktree/commit refs as durable result evidence.
- Workers can return S3-compatible object artifact refs for large generated outputs.
- Workers can return checkpoint refs for git-style task history, workspace snapshots, object archives, and metadata milestones.
- Operators can query checkpoint history through the goal store, control gateway, and MCP dashboard surface.
- Local Compose and Kubernetes development manifests include an S3-compatible object-store path, while AWS/EKS can use S3 through the same object-store contract.
- Production goal-store deployments use Postgres as the standard read model, with JSONB for exact payloads and optional pgvector for operational semantic search.
- Recurring work is modeled as event sources, triggered goals, Restate timers, or Kubernetes CronJobs, not worker-owned sleep loops.
- Actor state-machine tests cover valid transitions, invalid/stale recovery,
  terminal immutability, projection determinism, and rejection of placeholder
  satisfaction for non-stub goals.
