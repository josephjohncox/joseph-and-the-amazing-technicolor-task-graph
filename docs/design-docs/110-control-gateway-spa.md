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
- memory search, memory context packs, fork/join promotion, memory retraction/editing, semantic memory repair state, and research-output application.
- chat-assisted authoring for goals, plans, steering directives, state explanations, and JSON drafts.

The gateway must never own durable orchestration state. Restate remains the durable authority. The coordinator remains the only task-tree writer. The goal store, memory gateway, notifier, event gateway, and runner registry remain backend services.

## Components

- `ui/control-plane-web`: TypeScript gateway plus Vite/React SPA.
- `GET /`: browser operator UI.
- `GET /api/overview`: composed health, runner, notification, event, goal, and agent summary.
- `GET /api/approvals`: projected durable approval queue.
- `GET /api/follow-ups`: active execution-plan `## Follow-Ups` queue from `docs/exec-plans/active/`.
- `GET /api/goals`: goal-store projection list.
- `GET /api/plans`: durable planning-mode list.
- `POST /api/plans`: create a durable plan.
- `GET /api/plans/{plan_id}`: inspect a durable plan.
- `GET /api/plans/{plan_id}/continuity`: summarize durable plan questions, decisions, subgoals, initial tasks, revisions, and next actions.
- `POST /api/plans/{plan_id}/revisions`: append a plan revision.
- `POST /api/plans/{plan_id}/compile`: compile a plan into `GoalSpec`.
- `GET /api/goals/{goal_id}`: composed Restate plus goal-store snapshot.
- `POST /api/goals/submit`: submits a `GoalSpec` through `GoalWorkflow/run`.
- `POST /api/goals/{goal_id}/{handler}`: calls approved workflow handlers such as `steer`, `approve`, `cancel`, `restart`, `branch`, `select_branch`, `tasks`, `status`, and `progress`.
- `GET /api/agents`: projected task/agent rows across goals.
- `GET /api/human/threads`: local notification and feedback threads.
- `POST /api/chat`: chat assistant endpoint that can use a configured OpenAI-compatible chat-completions model or the local stub.
- `GET /api/events`, `/api/events/sources`, `/api/events/triggers`: event gateway read surfaces.
- `GET /api/memory/events/{goal_id}`: memory events projected for one goal.
- `POST /api/memory/{search,context,write,join,retract,edit,edit-preview,repair}`: memory gateway proxy.
- `POST /api/research/apply`: converts `ResearchOutput.use_plan` or an `InformationUsePlan` into coordinator-owned `SteeringDirective` calls.
- `POST /mcp`: MCP-compatible JSON-RPC surface for agent and chat clients.

## Engine Boundary

The browser UI is a real SPA, not HTML embedded in the Node server. The server
serves static Vite assets from `dist/public` and owns only gateway APIs, health,
MCP, and static-file delivery. Product pages live as React components under
`ui/control-plane-web/src/spa/`, with API access isolated in `src/spa/api.ts`
and styling in `src/spa/styles.css`.

Appearance is a first-class shell concern. The SPA provides a light, dark, and
system theme switcher, stores the operator preference locally, sets the
document color scheme before React boots, and themes React Flow, dialogs,
forms, cards, and status affordances through shared CSS variables.
The sidebar uses the COAT logo from the shared brand assets, while browser and
installed-app surfaces use compact icons generated from the same source artwork
plus the simplified technicolor task-graph mark.

The frontend stack is intentionally standard:

- Vite for TypeScript React bundling and production assets;
- React for product-facing pages and component composition;
- TanStack Query for server-state fetching, caching, refresh, and mutation state;
- React Flow for task-graph visualization;
- Radix primitives for accessible dialogs;
- lucide-react for iconography.

The SPA may edit text in browser forms, but edits become backend commands:

- new goal: `GoalWorkflow/run`;
- new/revised/compiled plan: `coat-goal-store` plan APIs;
- steering: `GoalWorkflow/steer`;
- approval: `GoalWorkflow/approve`;
- cancellation: `GoalWorkflow/cancel`;
- restart or branch selection: workflow handler;
- memory note: `coat-memory-gateway`;
- memory join or repair: `coat-memory-gateway`;
- memory retraction or replacement: `coat-memory-gateway`;
- research application: `GoalWorkflow/steer` directives derived from the sourced `InformationUsePlan`;
- event source or trigger: `coat-event-gateway`.

The chat tab is intentionally a drafting surface. It can read optional goal context and fill existing JSON forms, but durable mutations still require the operator to press the normal submit, steer, approve, memory, or plan buttons.

Chat provider configuration is optional:

- default: local stub drafts with no model call;
- OpenAI-compatible: set `COAT_CONTROL_CHAT_COMPLETIONS_URL`, `COAT_CONTROL_CHAT_MODEL`, and optionally `COAT_CONTROL_CHAT_API_KEY`;
- OpenAI account: set `COAT_CONTROL_CHAT_MODEL` and `OPENAI_API_KEY`; the gateway uses the OpenAI chat-completions URL.

It must not write Restate state, goal-store rows, runner rows, or memory storage directly.

## Agent Visibility

Agent state comes from projected `TaskRecord` rows plus the full `TaskNode` stored in `TaskRecord.payload_json`.

The UI should show:

- task ID, goal ID, parent task, subgoal, role, purpose, status, depth, priority, attempts, and runnable flag;
- current prompt from `payload_json.prompt`;
- execution profile, model route, persona, MCP refs, result channels, budget, sandbox profile, and done criteria;
- result refs, git refs, object artifacts, checkpoint history, recent task events, and child task IDs.

This is intentionally projection-based. If exact live state is needed, the gateway also calls `GoalWorkflow/status` and `GoalWorkflow/progress`; the UI should label stale or failed projection reads instead of pretending they are authoritative.

## Follow-Up Queue

Execution plans keep durable continuation items under `## Follow-Ups`.

The gateway exposes those items through:

- the overview dashboard;
- a dedicated Follow-Ups tab with search, counts, source paths, raw projection inspection, and chat-assisted plan drafting;
- `GET /api/follow-ups`;
- `POST /api/follow-ups/draft-plan`;
- MCP tool `coat_follow_ups`.
- MCP tool `coat_follow_up_draft_plan`.

The SPA also groups projected plans by `source_plan_id` so operators can compare branched planning candidates and select one as the current plan before compilation.

This is a documentation projection, not workflow state. Operators and agent clients should use it to choose the next implementation slice, then turn work into durable plans, goals, steering directives, or code changes through the normal backend APIs.

## Human Queue

The human queue is a consolidated view over notification threads and workflow approval/feedback state.

Supported local queue items:

- approval requests;
- feedback requests;
- blocked tasks;
- failures;
- completion notifications;
- async-response requests such as device-code login, source clarification, or branch-selection review.

Production notification adapters can route the same `NotificationRequest` records to Slack, email, trackers, webhooks, paging, or a future chat assistant. The local notifier already exposes dashboard queue records, Slack incoming webhooks, generic webhooks, tracker webhook payloads, PagerDuty Events API v2 delivery, and a structured email outbox. Human decisions still resume the workflow through durable handlers.

## MCP And Other Dashboards

The gateway exposes MCP tools so agent/chat clients can inspect and steer the system without using the SPA:

- `coat_overview`;
- `coat_goal_snapshot`;
- `coat_agent_activity`;
- `coat_plan_list`;
- `coat_plan_draft`;
- `coat_plan_get`;
- `coat_plan_revise`;
- `coat_plan_continuity`;
- `coat_plan_compile`;
- `coat_follow_ups`;
- `coat_goal_submit`;
- `coat_human_threads`;
- `coat_approval_queue`;
- `coat_approve_goal`;
- `coat_steer_goal`;
- `coat_chat_assist`;
- `coat_runner_list`;
- `coat_runner_register`;
- `coat_memory_search`;
- `coat_memory_context`;
- `coat_memory_write`;
- `coat_memory_join`;
- `coat_memory_retract`;
- `coat_memory_edit`;
- `coat_memory_edit_preview`;
- `coat_memory_repair`;
- `coat_memory_events`;
- `coat_apply_research_output`;
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
