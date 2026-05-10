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
- chat-assisted authoring for goals, plans, backend-routed search requests, steering directives, state explanations, and JSON drafts.

The gateway must never own durable orchestration state. Restate remains the durable authority. The coordinator remains the only task-tree writer. The goal store, memory gateway, notifier, event gateway, and runner registry remain backend services.

## Components

- `ui/control-plane-web`: TypeScript gateway plus Vite/React SPA.
- `GET /`: browser operator UI.
- `GET /api/overview`: composed health, runner, notification, event, goal, and agent summary.
- `GET /api/approvals`: projected durable approval queue.
- `GET /api/follow-ups`: compatibility projection for durable plan-continuity next actions from goal-store plans.
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
- `GET /api/runners`: normalized runner-registry status rows with top-level `runner_id`, `node_id`, `endpoint`, `display_name`, status, capacity, labels, roles, capabilities, and model candidates for fleet UI and MCP clients.
- `GET /api/human/threads`: local notification and feedback threads.
- `POST /api/chat`: backend chat assistant endpoint. The browser posts only to the control gateway; the gateway either uses explicit chat-completions config, discovers a dispatchable OpenAI-compatible/local model from the runner registry, or falls back to the local stub. User and assistant turns are journaled through the goal-store chat-turn API when available.
- `GET /api/chat/session`: read a durable chat session for the selected goal or operator workspace.
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
- Chatscope React components for the chat composer, message list, send behavior, and typing state;
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

The chat tab is intentionally a drafting surface. The browser must not call model providers directly. It posts prompts to `/api/chat`; the control gateway resolves an operator-chat backend from gateway configuration, calls the provider server-side, journals the user and assistant turns, and returns draft payloads. This lookup is for chat assistance on a user request; it is not durable task dispatch and must not call runner `/run-task` APIs. It can read optional goal context and fill existing JSON forms, but durable mutations still require the operator to press the normal submit, steer, approve, memory, or plan buttons.

Chat supports three primary authoring modes: durable plan draft, GoalSpec draft, and search request. Search mode emits a structured `search_request` plus an optional coordinator-owned research steering directive. It must not claim live memory, web, or reference search occurred unless a backend tool result is present.

Every `/api/chat` request carries a `run_id`. The gateway records a short-lived operational trace with real stages such as goal-context load, backend resolution, model call or stub drafting, and chat-turn journaling. The SPA exposes this as "Chat activity"; it is an operational trace, not hidden model reasoning.

Chat history is server-side state. The gateway first writes turns to `coat-goal-store` through `POST /goal-store/chat/turns`, so Compose can keep local JSONL or use the standard Postgres read model behind the same API. `COAT_CONTROL_CHAT_STORE_BACKEND=goal_store` is the default. `COAT_CONTROL_CHAT_JOURNAL_PATH` is a local fallback for smoke tests, single-node development, or when goal-store is temporarily unavailable. A gateway replica must never rely on browser local storage as the durable conversation log.

Chat provider configuration is optional:

- default: `COAT_CONTROL_CHAT_BACKEND=configured`, meaning explicit gateway chat config or local stub drafts with no model call;
- shared LLM gateway: set `COAT_LLM_GATEWAY_URL`,
  `COAT_LLM_GATEWAY_CHAT_MODEL` or `COAT_LLM_GATEWAY_DEFAULT_MODEL`, and
  optionally `COAT_LLM_GATEWAY_API_KEY`; this is the preferred path for
  Bifrost, LiteLLM, OpenRouter, Docker Model Gateway, or another
  OpenAI-compatible gateway that owns provider keys and routing;
- OpenAI-compatible: set `COAT_CONTROL_CHAT_PROVIDER=openai_compatible`, `COAT_CONTROL_CHAT_COMPLETIONS_URL`, `COAT_CONTROL_CHAT_MODEL`, and optionally `COAT_CONTROL_CHAT_API_KEY`;
- OpenAI account: set `COAT_CONTROL_CHAT_PROVIDER=openai`, `COAT_CONTROL_CHAT_MODEL`, and `OPENAI_API_KEY` or `COAT_CONTROL_CHAT_API_KEY`; the gateway uses the OpenAI chat-completions URL.
- runner-registry discovery: set `COAT_CONTROL_CHAT_BACKEND=runner_registry` when operator chat should intentionally borrow a dispatchable registered runner model with an OpenAI-compatible endpoint, such as an Ollama, vLLM, or llama.cpp `/v1` endpoint. The runner or model candidate must be explicitly labeled for operator chat with labels such as `control_chat=true`, `chat.intent=user_request`, or `routing_scope=operator_chat`; unlabeled durable-work runners are ignored.

Gateway chat and runner models are intentionally separate. Runner models are capacity for durable task work and are steered by `TaskNode.execution`, worker role, persona, runner labels, sandbox profile, and model route. Gateway chat and memory embeddings are operator UX defaults selected through `COAT_CONTROL_CHAT_*`, `COAT_LLM_GATEWAY_*`, and `MEMORY_GATEWAY_EMBEDDING_*`. A local model runner can exist for durable tasks while the gateway Chat tab uses OpenAI, or the inverse; neither path should silently infer the other.

Optional runtime params are `COAT_CONTROL_CHAT_SPEED_TIER`,
`COAT_CONTROL_CHAT_TEMPERATURE`, `COAT_CONTROL_CHAT_TOP_P`,
`COAT_CONTROL_CHAT_MAX_OUTPUT_TOKENS`, `COAT_CONTROL_CHAT_REASONING_EFFORT`,
and `COAT_CONTROL_CHAT_TIMEOUT_SECONDS`. The setup wizard exposes fast, balanced,
speed-tier, deep-review, deterministic, and custom choices instead of requiring
operators to remember these keys. Hosted-only fields such as `reasoning_effort`
and `service_tier` are sent only to hosted OpenAI chat endpoints; local
OpenAI-compatible runner-discovered endpoints receive portable sampling and
token-limit parameters.

## Search Enablement

The implemented search path is backend-owned memory search:

- SPA Memory view calls `/api/memory/search` and `/api/memory/context`;
- gateway MCP exposes `coat_memory_search` and `coat_memory_context`;
- `coat-memory-gateway` owns local lexical search plus optional Graphiti/Zep and Qdrant-backed retrieval.

Chat Search mode is an authoring path. It drafts a `search_request` and a research-task steering directive so the coordinator can decide whether to run memory, docs, reference, or web search under the goal `research_policy`.

Web/reference search should be enabled as a standard tool-registry/MCP capability, not as UI code. The intended production shape is a configured `web_search` tool in `coat-tool-registry`, proxied by gateway MCP as `coat_web_search`, with an approved external search gateway, token/secret refs, source capture, and disabled-by-default behavior when no search gateway is configured.

Codex and Claude Code may also have native search surfaces. Treat those as runner
capabilities, not as the control-plane search contract:

- repository/workspace search through local commands such as `rg`, `git grep`, and
  language tooling is normal executor capability and should be declared through
  `ExecutionProfile.local_tools`;
- Claude Code can expose web search when the tool and organization policy allow
  it;
- Codex/OpenAI harnesses may expose web/search tools depending on the active
  product surface, sandbox, network policy, and configured tool set;
- local vLLM, Ollama, llama.cpp, Bedrock, and other provider runners should not
  be assumed to have native web search.

When a runner uses native search, it must still return structured
`ResearchOutput`, `SourceArtifact` evidence, provenance, and an
`InformationUsePlan`. The coordinator should route native-search work only to
runners that advertise capabilities such as `native_web_search`,
`provider_web_search`, `repo_search`, or `docs_search`. The portable default
remains `coat_memory_search`/`coat_memory_context` plus the future
`coat_web_search` MCP tool so search behavior is auditable and consistent across
runner types.

The gateway must not write Restate state, runner rows, or memory storage directly. It may write gateway-owned projection records, such as chat turns, only through documented backend APIs.

## Agent Visibility

Agent state comes from projected `TaskRecord` rows plus the full `TaskNode` stored in `TaskRecord.payload_json`.

The UI should show:

- task ID, goal ID, parent task, subgoal, role, purpose, status, depth, priority, attempts, and runnable flag;
- current prompt from `payload_json.prompt`;
- execution profile, model route, persona, MCP refs, result channels, budget, sandbox profile, and done criteria;
- result refs, git refs, object artifacts, checkpoint history, recent task events, and child task IDs.

This is intentionally projection-based. If exact live state is needed, the gateway also calls `GoalWorkflow/status` and `GoalWorkflow/progress`; the UI should label stale or failed projection reads instead of pretending they are authoritative.

## Continuation Work

User-facing continuation work comes from standard backend records:

- durable plans and plan-continuity `next_actions`;
- goal progress and task state;
- event and trigger projections;
- human queue approvals, blocked tasks, feedback requests, and async-response requests.

Repo markdown `## Follow-Ups` remains a developer doc-gardening convention for active execution plans. It is not the product queue.

The gateway keeps `GET /api/follow-ups`, `POST /api/follow-ups/draft-plan`, MCP `coat_follow_ups`, and MCP `coat_follow_up_draft_plan` as compatibility surfaces, but their operator-facing semantics are durable plan-continuity next actions. New UI work should prefer Plans, Goals, Events, and Human Queue views.

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
