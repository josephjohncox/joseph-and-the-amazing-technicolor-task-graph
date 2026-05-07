# Control Gateway And SPA

## Purpose

The control gateway is an operator surface over the engine, not a second engine.

It exists so a human or another agent can see and steer:

- goal state and progress;
- durable planning-mode drafts and compiled plan output;
- task and agent state;
- current projected prompts and task contracts;
- runner capacity and model-routing state;
- notification, approval, feedback, and async-response queues;
- event sources, cron-like triggers, webhooks, calendars, and recent external events;
- memory search, memory context packs, and semantic memory repair state.

The gateway must never own durable orchestration state. Restate remains the durable authority. The coordinator remains the only task-tree writer. The goal store, memory gateway, notifier, event gateway, and runner registry remain backend services.

## Components

- `ui/control-plane-web`: TypeScript gateway and SPA.
- `GET /`: browser operator UI.
- `GET /api/overview`: composed health, runner, notification, event, goal, and agent summary.
- `GET /api/approvals`: projected durable approval queue.
- `GET /api/goals`: goal-store projection list.
- `GET /api/plans`: durable planning-mode list.
- `POST /api/plans`: create a durable plan.
- `GET /api/plans/{plan_id}`: inspect a durable plan.
- `POST /api/plans/{plan_id}/revisions`: append a plan revision.
- `POST /api/plans/{plan_id}/compile`: compile a plan into `GoalSpec`.
- `GET /api/goals/{goal_id}`: composed Restate plus goal-store snapshot.
- `POST /api/goals/submit`: submits a `GoalSpec` through `GoalWorkflow/run`.
- `POST /api/goals/{goal_id}/{handler}`: calls approved workflow handlers such as `steer`, `approve`, `cancel`, `restart`, `branch`, `select_branch`, `tasks`, `status`, and `progress`.
- `GET /api/agents`: projected task/agent rows across goals.
- `GET /api/human/threads`: local notification and feedback threads.
- `GET /api/events`, `/api/events/sources`, `/api/events/triggers`: event gateway read surfaces.
- `POST /api/memory/{search,context,write,join,repair}`: memory gateway proxy.
- `POST /mcp`: MCP-compatible JSON-RPC surface for agent and chat clients.

## Engine Boundary

The SPA may edit text in browser forms, but edits become backend commands:

- new goal: `GoalWorkflow/run`;
- new/revised/compiled plan: `coat-goal-store` plan APIs;
- steering: `GoalWorkflow/steer`;
- approval: `GoalWorkflow/approve`;
- cancellation: `GoalWorkflow/cancel`;
- restart or branch selection: workflow handler;
- memory note: `coat-memory-gateway`;
- event source or trigger: `coat-event-gateway`.

It must not write Restate state, goal-store rows, runner rows, or memory storage directly.

## Agent Visibility

Agent state comes from projected `TaskRecord` rows plus the full `TaskNode` stored in `TaskRecord.payload_json`.

The UI should show:

- task ID, goal ID, parent task, subgoal, role, purpose, status, depth, priority, attempts, and runnable flag;
- current prompt from `payload_json.prompt`;
- execution profile, model route, persona, MCP refs, result channels, budget, sandbox profile, and done criteria;
- result refs, git refs, object artifacts, recent task events, and child task IDs.

This is intentionally projection-based. If exact live state is needed, the gateway also calls `GoalWorkflow/status` and `GoalWorkflow/progress`; the UI should label stale or failed projection reads instead of pretending they are authoritative.

## Human Queue

The human queue is a consolidated view over notification threads and workflow approval/feedback state.

Supported local queue items:

- approval requests;
- feedback requests;
- blocked tasks;
- failures;
- completion notifications;
- async-response requests such as device-code login, source clarification, or branch-selection review.

Production notification adapters can route the same `NotificationRequest` records to Slack, email, trackers, webhooks, paging, or a future chat assistant. Human decisions still resume the workflow through durable handlers.

## MCP And Other Dashboards

The gateway exposes MCP tools so agent/chat clients can inspect and steer the system without using the SPA:

- `coat_overview`;
- `coat_goal_snapshot`;
- `coat_agent_activity`;
- `coat_plan_list`;
- `coat_plan_get`;
- `coat_plan_compile`;
- `coat_human_threads`;
- `coat_approval_queue`;
- `coat_steer_goal`;
- `coat_memory_search`;
- `coat_event_sources`.

External dashboards should prefer the same gateway APIs or the lower-level backend APIs. The gateway is useful for composition and auth consolidation, while the lower-level services remain the stable engine contracts.

## Auth

Set `COAT_CONTROL_GATEWAY_TOKEN` to require bearer auth for `/api/*`.

Set `COAT_CONTROL_MCP_TOKEN` to require bearer auth for `/mcp`; when unset it falls back to `COAT_CONTROL_GATEWAY_TOKEN`.

Backend tokens are still resolved by the gateway from environment or Kubernetes Secret refs:

- `COAT_EVENT_GATEWAY_TOKEN`;
- `COAT_MEMORY_GATEWAY_TOKEN`;
- service URLs for Restate, goal store, event gateway, notifier, runner registry, and memory gateway.

The gateway must not echo backend tokens in `/api/config`, MCP results, diagnostics, or UI output.

## Deployment

Compose starts `control-web` on `:9090`.

Kubernetes deploys `control-web` as an independent Deployment and Service. It can sit behind ingress/TLS, OAuth proxy, VPN, or private network access.

The gateway is optional. Removing it must not affect coordinator execution, runner dispatch, event ingestion, validation, or memory operation.
