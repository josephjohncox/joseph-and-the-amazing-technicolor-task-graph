# 160 Live Durable Runtime And Execution

## Objective

Coordinate the remaining live-runtime work across Restate durability, live workers, Kubernetes executors, memory, research, events, notifications, UI, release proof, and generated protocols.

This is the cross-cutting execution plan for the high-value follow-ups preserved from the completed subsystem plans. It does not reopen those plans; it sequences their residual proof work so the system moves from scaffold to proven durable runtime without duplicating subsystem ownership.

Backend-first simplification work also rolls up here. The product model is a durable actor-style task graph: Restate owns orchestration, coordinator handlers own state transitions, Postgres or the local JSONL goal-store backend owns the operator-facing read model, `/api/operator/*` is the compact mutation/read surface, `/api/operator/stream` is the projection stream, and SPA/TUI clients render the same current-goal, action queue, evidence, worker-run, event, and graph projections.

## Defaults

- Restate harness: Docker-backed restart/resume proof with a pinned Restate
  image. The harness may move to the Rust Testcontainers crate later if that
  buys cleaner CI lifecycle management, but direct Docker proof is sufficient
  for the current runtime evidence gate.
- First live worker: Codex App Server.
- First Kubernetes proof: kind or k3d CI.
- Live test policy: env-gated live tests plus replay fixtures that always run in CI.
- First event and notification proof: SQS through LocalStack.
- Plan shape: one master active plan linked from subsystem plans.

## Subsumed Plans

The completed plans under `docs/exec-plans/completed/` remain the subsystem evidence record. Their residual live-runtime follow-ups are preserved here so `docs/exec-plans/active/` can stay focused on this single coordination plan.

- `000-bootstrap-harness`: repo and doc harness are complete; ongoing doc gardening stays as normal maintenance.
- `010-domain-task-tree`: core domain contracts are complete; future lifecycle contract growth is handled through this plan's protocol and test gates.
- `020-restate-coordinator`: coordinator scaffold is complete; real restart/resume and observability proof moves to `RuntimeVerifier`.
- `030-codex-worker`: Codex stub, registration, verification, and result refs are complete; live execution moves to `CodexWorker`.
- `040-staff-engineer-worker`: staff-engineer stub routing is complete; live package verification and issue-to-PR smoke run after Codex proves the live worker contract.
- `050-research-worker`: research contracts and replay fixture shape are complete; live source capture moves to `ResearchMemory`.
- `060-test-review-validator`: validator/reviewer contracts are complete; real live worker outputs become new reviewer fixtures through `Reviewer`.
- `070-sandbox-tooling`: sandbox/tool contracts are complete; Kubernetes execution and object upload promotion move to `Provisioner`.
- `080-deploy-and-cli`: deploy and CLI scaffold is complete; release proof and executor provisioning move to `ReleaseHardening` and `Provisioner`.
- `090-distributed-runners-mcp-notifications`: runner/MCP/notification contracts are complete; live provider profiles and SQS proof move to `CodexWorker` and `EventOps`.
- `100-steering-research-memory`: steering and memory contracts are complete; live adapters and browser memory workflow move to `ResearchMemory` and `UIE2E`.
- `110-protobuf-goal-store`: protobuf and goal-store projection are complete; SDK generation and Restate restart proof move to `ProtocolSDK` and `RuntimeVerifier`.
- `120-events-webhooks-schedules`: event gateway contracts are complete; provider adapters and topology proof move to `EventOps`.
- `130-restate-cloud-personal-corporate`: Restate Cloud support is complete; provider overlays and journal encryption guidance move to `ReleaseHardening`.
- `140-control-gateway-spa`: gateway and SPA scaffold are complete; full Compose browser E2E and token-broker smoke move to `UIE2E`.
- `150-durable-planning-mode`: durable planning mode is complete with no residual follow-ups.

## Durable Child Task Workstreams

- `RuntimeVerifier`: owns Restate restart/resume tests, durable projection idempotency, metrics, and traces.
- `CodexWorker`: owns live Codex App Server execution, Codex MCP fallback, and replayable fixtures.
- `Provisioner`: owns coordinator-approved Kubernetes executor Jobs, Job watching, result ingestion, and attestations.
- `ResearchMemory`: owns live research, source capture, Qdrant, Graphiti/Zep, and object-store snapshots.
- `EventOps`: owns SQS/LocalStack inbound and outbound proof, notifier outbox, retries, acknowledgements, and DLQ behavior.
- `UIE2E`: owns browser-level Compose workflows for goals, task graph, memory edits, approvals, runners, and event sources.
- `ReleaseHardening`: owns GitHub Release, Helm chart, kind/k3d, and published smoke evidence.
- `ProtocolSDK`: owns Buf-generated Rust and TypeScript SDK target selection and generation.
- `Reviewer`: reviews each workstream for correctness, security, testing depth, and public-contract drift.
- `Unifier`: joins accepted workstream outputs and decides whether the master plan can be moved to completed.

Workers in these workstreams use COAT durable child tasks. They must not use hidden native Codex, Claude Code, Agents SDK, or MCP subagent spawning. Any request for more work returns `ChildTaskRequest` values for coordinator approval.

## Run-To-Completion Simplification Plan

This section is the durable execution plan for the remaining refactoring,
operator-product cleanup, and simplification work. It intentionally lives inside
the single active master plan so follow-up work does not scatter back across the
completed subsystem plans.

PLAN-1 is the authoritative simplification design. PLAN-0 contributes only
non-conflicting implementation organization, especially module boundaries,
projection builders, service cleanup, and model-based state-machine tests. When
the two differ, prefer PLAN-1: backend-first actor state machines, compact
operator APIs, Postgres or local goal-store read projection, SSE as projection
streaming, Vite/React/shadcn SPA, Ratatui TUI, and bounded registered workers.

### Core Goal

Make COAT feel like a direct operator console for a durable task graph:

- The operator can create, inspect, steer, block, resume, approve, reject,
  cancel, and complete goals without understanding Restate URLs, raw workflow
  IDs, JSON payload shapes, or internal compatibility surfaces.
- The SPA and TUI answer the same questions in the same language: what goal is
  selected, what is running, what is blocked, what action is needed, what
  evidence exists, what workers are active, and whether the goal is satisfied.
- Backend state transitions stay actor-style and typed. UI surfaces render and
  command backend state; they do not own durable truth.
- Chat is an authoring and explanation surface. Draft acceptance, human prompts,
  approvals, recovery, branch selection, and cancellation are explicit operator
  actions, not buried chat replies.
- Remaining live-runtime work continues, but the refactor priority is to make
  the normal local/operator experience simple before adding more visible knobs.

### Product Model

The operator product model has five primary objects:

- `Goal`: the selected durable objective, with status, satisfaction, progress,
  and evidence.
- `Task`: a bounded work item owned by the coordinator and executed by one
  registered worker.
- `Action`: a human or coordinator-visible decision needed to make progress,
  such as approve, reject, continue, answer, add context, retry, replan, select
  branch, restart, or cancel.
- `Run`: worker execution history, output, checkpoint refs, errors, and current
  stage.
- `Evidence`: artifacts, checkpoints, reviews, test reports, citations,
  approvals, and satisfaction rationale.

Everything else is secondary drill-down:

- thunks are action-producing suspended continuations;
- reviews and adversarial rounds are evidence and decision workflows;
- events are external signals that create or steer goals;
- memory is context and provenance, not a substitute for evidence;
- debug payloads are inspectable, but not the primary interface.

### Interaction Target

The SPA first screen should be a focused goal workbench:

- top bar: workspace health, current-goal switcher, global search, theme, and
  one debug/inspect affordance;
- left column: goal list and current action queue;
- center: selected goal summary, task graph, and next action;
- right column: chat/draft panel, evidence/review panel, and worker run stream;
- contextual drawers: memory, events, adversarial/review rounds, raw inspect.

The TUI should mirror the same model:

- tabs: Overview, Goals, Graph, Actions, Approvals, Events, Workers, Evidence,
  Adversarial, Debug;
- one selected goal inherited by every tab and by chat;
- enter/escape/tab/shift-tab behavior must be predictable;
- chat input clears on submit, pending work is visible, history scrolls, and the
  operator can submit or discard drafts without switching to the CLI.

### Refactor Principles

- No compatibility work for removed UI/helper surfaces unless a live operator
  path still depends on them.
- Prefer deletion over hiding when a control does not help answer the selected
  goal's current state.
- Keep all mutations on compact typed backend APIs under `/api/operator/*`.
- Keep lower-level Restate, goal-store, and raw JSON views behind inspect/debug.
- Make impossible states boring: invalid transitions must be rejected with a
  recovery action, and the UI must surface that recovery action.
- A blocked state must be actionable. It needs either a resumable thunk or
  explicit retry, restart, replan, create-human-prompt, add-context, or cancel
  controls.
- A completed goal must show evidence and satisfaction rationale.
- A draft is a server-owned resource with lifecycle state: active, edited,
  accepted, submitted, discarded, or expired.

### State Machine Simplification

The core cleanup is to make the backend state machines small, explicit, and
testable. Avoid scattering lifecycle rules across coordinator handlers,
goal-store projection code, gateway normalization, SPA reducers, and TUI state.

Authoritative actor kinds:

- `GoalActor`
- `TaskActor`
- `ThunkActor`
- `WorkerRunActor`
- `ReviewActor`
- `ApprovalActor`
- `EventActor`
- `DraftActor`
- `RunnerActor`
- `MemoryActor`
- `MechanismActor`

Authoritative lifecycle classes:

- active: runnable, dispatching, running, reviewing, validating;
- waiting: waiting-input, waiting-approval, waiting-event, waiting-resource,
  waiting-model, waiting-timer;
- recoverable: blocked, failed, timeout, stale, budget-exhausted,
  validation-needed;
- terminal: satisfied, cancelled;
- immutable history: archived events, delivered notifications, accepted
  evidence, completed worker runs.

Required typed transitions:

- submit goal, accept draft, edit draft, discard draft, submit draft as goal;
- dispatch task, receive worker result, validate result, request review,
  complete review, select branch, satisfy goal;
- create thunk, resume thunk, expire thunk, cancel thunk;
- request approval, approve, reject, request more context;
- steer goal, replan goal, retry task, restart goal, cancel goal;
- ingest event, dedupe event, route event to goal, dead-letter event;
- register runner, heartbeat runner, drain runner, mark runner stale;
- write memory evidence, retract memory entry, repair memory adapter replay;
- start mechanism round, submit ballot or bid, close round, apply outcome.

Simplification rules:

- Each transition has one backend implementation and one recovery path for
  invalid or stale inputs.
- UI and TUI never infer lifecycle validity. They render
  `OperatorAction.available_actions` and send typed action requests.
- A non-terminal actor with no available action is invalid unless it is actively
  running under a live `WorkerRunActor`.
- A terminal actor can still be inspected but cannot be steered, restarted, or
  mutated except through explicit archival or fork/new-goal flows.
- The read model projects from durable events and actor snapshots. It must not
  manufacture success states from placeholder text.
- Service-specific states map into these actor classes. Do not build separate
  hidden state machines in the gateway, SPA, TUI, runner registry, notifier, or
  event gateway.

Implementation organization from PLAN-0, when it does not conflict with PLAN-1:

- Split `crates/domain/src/lib.rs` into focused modules while preserving public
  exports where current code still imports the root module:
  `goal`, `task`, `state_machine`, `validation`, `approval`, `thunk`, `review`,
  `mechanism`, `runner`, `memory`, `events`, `operator_projection`, and
  `schemas`.
- Refactor `crates/coordinator/src/main.rs` into workflow handlers, mutation
  handlers, transition helpers, dispatch loop, projection emission, service
  clients, and tests.
- Refactor `ui/control-plane-web/src/server.ts` into routes, service clients,
  chat, drafts, operator projection, operator actions, MCP tools, and SSE.
- Refactor goal-store, event-gateway, sandbox-runner, memory-gateway, notifier,
  and runner-registry internals around repositories/adapters/projections rather
  than endpoint-local JSON shaping.
- Add projection builders for `GoalState -> OperatorWorkspaceSnapshot`, action
  queue projection from approvals/thunks/compute graph, actor/critic projection
  from reviews and mechanism rounds, and runner/event/memory summary rows.

### State Machine Testing

The state-machine test suite should prove lifecycle behavior, not just enum
serialization.

Required test layers:

- table-driven transition tests for every valid transition and every expected
  invalid transition;
- model-based/property tests, preferably with `proptest`, that generate bounded
  transition sequences and assert global invariants after each step;
- projection rebuild tests that replay append-only events into the same actor
  snapshot and action queue;
- stale-action tests proving repeated, late, or already-resolved actions return
  explicit recovery results without wedging the workflow;
- scenario tests that exercise lifecycle stories through backend APIs rather
  than direct state mutation;
- SPA/TUI tests that prove the same selected actor, available actions, blocker,
  evidence, and satisfaction state are visible through both clients.

Global invariants:

- satisfied and cancelled are the only terminal goal states;
- blocked, failed, timeout, stale, waiting, budget-exhausted, and
  validation-needed states always expose a recovery action;
- every waiting human prompt has either a delayed compute thunk with a
  continuation ref or an explicit recovery action to create one;
- every worker run belongs to exactly one task and has at most one terminal
  result;
- every evidence item has a causation actor, correlation ID, and provenance ref;
- every branch selection references validated candidate evidence;
- every event has an idempotency key and dedupe outcome;
- every action resolution is idempotent by action ID and causation ID;
- replaying the same durable event sequence yields the same operator projection;
- placeholder/stub results cannot satisfy non-stub goals.

### Execution Phases

#### Phase 0: Product Cut And Contract Freeze

- Inventory the currently exposed SPA routes, TUI tabs, CLI commands, MCP tools,
  gateway endpoints, and scenario fixtures.
- Mark each surface as keep, fold into another surface, debug-only, or delete.
- Freeze the operator projection contract for the next implementation pass:
  workspace, goals, goal detail, graph, actions, events, evidence, workers,
  drafts, chat sessions, and stream events.
- Record the PLAN-1/PLAN-0 incorporation decision: PLAN-1 owns product and
  public contracts; PLAN-0 contributes only non-conflicting module and test
  organization.
- Freeze the actor transition matrix and global invariants before deep SPA/TUI
  refactors so clients target a stable action model.

Exit criteria:

- one route/tab map for SPA and TUI;
- one API contract checklist;
- one deletion list for confusing controls and legacy surfaces;
- one actor transition matrix and invariant checklist;
- no UI implementation begins until backend action semantics for that route are
  stable or explicitly stubbed as unavailable.

#### Phase 1: Backend State And Projection Cleanup

- Add or tighten the small domain actor/state-machine layer for goals, tasks,
  thunks, worker runs, reviews, approvals, events, drafts, runners, memory, and
  mechanism rounds.
- Collapse scattered lifecycle helpers into typed transition functions with
  `TransitionResult` values that include accepted, rejected, stale, no-op, and
  recovery guidance.
- Normalize action queue records so approvals, thunks, blocked tasks, failed
  tasks, branch decisions, adversarial decisions, and cancel/restart actions all
  share one product-shaped `OperatorAction`.
- Ensure goal-store projections rebuild the same action queue from append-only
  events and snapshots.
- Persist chat sessions and drafts in the goal-store/Postgres path rather than
  relying on in-memory gateway maps for durable UX.
- Add `GET /api/operator/goals/:id/timeline` or fold timeline into goal detail
  so the SPA/TUI can show why a goal is blocked.
- Keep SSE as projection streaming only; no mutation over SSE.
- Standardize mutation routing through the compact operator action path:
  `POST /api/operator/actions/:id/resolve` for existing actions and, where a
  command creates a new action-like transition, a typed operator action request
  that returns `OperatorActionResult` plus refreshed projection state.
- Split domain, coordinator, gateway, and service modules where file size or
  duplicated lifecycle logic prevents state-machine reasoning.

Exit criteria:

- action queue projection has unit tests for every action kind;
- actor transition tests cover valid, invalid, stale, and idempotent paths;
- projection rebuild tests prove event-log replay produces stable operator
  snapshots;
- draft lifecycle survives gateway restart when goal-store/Postgres is enabled;
- blocked goal detail shows blocker, reason, recovery action, and evidence refs.

#### Phase 2: SPA Simplification

- Split the SPA into feature modules around the product model:
  `GoalWorkbench`, `GoalSwitcher`, `ActionQueue`, `TaskGraph`, `EvidencePanel`,
  `WorkerRuns`, `ChatDraftPanel`, `EventsPanel`, `MemoryPanel`, and
  `DebugInspector`.
- Use shadcn-backed components for cards, buttons, dialogs, command picker,
  tabs, sheets, forms, and toasts. Keep Vite and React.
- Remove normal raw UUID entry, command coverage panels, endpoint taxonomy, and
  broad control menus from the main flow.
- Make the chat panel mode simple:
  `Ask`, `Draft goal`, `Draft plan`, and `Search/research request`.
  The selected goal context is visible, but not a dropdown maze.
- Render drafts as compact editable summaries with primary actions:
  edit, accept, submit, convert to plan, discard.
- Render human prompts as direct controls:
  continue, answer, add context, approve, reject, retry, replan, cancel.
- Make graph nodes clickable and side-panel driven: node state, next action,
  worker run, evidence, and raw inspect.

Exit criteria:

- an operator can create a goal from chat, accept the draft, see it selected,
  inspect subgoals/tasks, resolve a prompt, and cancel/restart without pasting an
  ID;
- the UI has fewer top-level options than today while exposing all common
  actions where they are needed;
- Playwright tests assert visible outcomes, not just button presence.

#### Phase 3: TUI Simplification

- Keep Ratatui/Crossterm unless a specific terminal framework prototype proves a
  materially better chat/input experience without splitting backend semantics.
- Make tab focus and chat focus deterministic:
  `Tab`/`Shift-Tab` change panels, `Enter` focuses or activates the selected
  row, and `Ctrl-Enter` or configured send key submits chat if needed.
- Add scroll indicators for chat, actions, graph rows, and evidence.
- Add explicit draft card controls and action-row controls in the TUI:
  accept draft, discard draft, approve, reject, continue, retry, replan, cancel.
- Keep raw JSON only in Debug.

Exit criteria:

- TUI tests cover goal selection, chat scrolling, draft submit/discard, action
  resolution, approval reject/approve, cancel/restart, and selected-goal context;
- manual TUI smoke can complete the same basic lifecycle as the SPA.

#### Phase 4: Scenario And Usability Proof

- Expand deterministic scenarios into use-case stories:
  basic lifecycle, blocked/resumed, pending action, approval reject/approve,
  signal-driven goal, fanout, fork/join review, long iterative loop, cancel and
  queue cleanup, memory/research evidence.
- Treat the scenario scripts as the main system exercise harness, not incidental
  CI helpers. The harness must reset, seed, drive, observe, and report on real
  product flows through the same backend APIs used by SPA, TUI, CLI, and event
  gateways.
- Add browser-visible checks for the main workbench: selected goal, action
  queue, task graph, evidence, worker runs, draft state, and completion.
- Add TUI transcript capture once the scenario harness can drive terminal
  surfaces deterministically.
- Keep LLM usability evaluation optional and gated. Deterministic rubrics remain
  the PR gate.

Exit criteria:

- `make scenario-e2e` and `make scenario-e2e-ui` prove operator outcomes;
- failed scenarios upload enough evidence for a reviewer to understand the user
  failure without reproducing locally.

### Scenario Scripts And System Exercise Harness

The simplification pass needs executable scripts that make the system feel real
locally and provide reviewer evidence in CI. These scripts should exercise COAT
as an operator product: reset state, seed useful scenarios, run bounded work,
show pending human actions, complete recoverable paths, and emit a report with
links to evidence.

Keep these existing entrypoints as the base:

- `scripts/coat-scenario-e2e.sh`: deterministic backend scenario runner.
- `scripts/coat-bootstrap-scenarios.sh`: fixture-backed bootstrap and seed
  generation.
- `scripts/coat-bootstrap-live-scenarios.sh`: live local-stack bootstrap goals.
- `scripts/coat-local-reset.sh`: scenario, bootstrap, evidence, and stack reset.
- `scripts/coat-event-gateway-smoke.sh` and
  `scripts/coat-event-gateway-compose-smoke.sh`: event ingress exercise.
- `scripts/coat-runner-registry-smoke.sh` and
  `scripts/coat-compose-runner-smoke.sh`: runner registration and routing
  exercise.
- `scripts/coat-eventops-sqs-smoke.sh`: SQS/LocalStack notification and event
  queue exercise.

Add or standardize one top-level exercise wrapper:

- `scripts/coat-exercise-system.sh`

It should provide these modes:

- `--mode quick`: reset syntax/dry-run smoke, deterministic bootstrap
  fixtures, runner-registry smoke, and event-gateway smoke without directly
  starting the Compose stack.
- `--mode demo`: reset local demo evidence, start or reuse local services, seed
  navigable goals for completed, running, pending-action, approval, blocked,
  fanout, fork/join, signal, memory/research, and cancelled-history states.
- `--mode e2e`: run the deterministic stub-stack scenario suite through
  `make scenario-e2e`.
- `--mode ui`: run the deterministic stack plus Playwright operator journeys
  through `make scenario-e2e-ui` or `make scenario-e2e-ui-live`.
- `--mode full`: run quick, demo, backend scenarios, UI scenarios, event gateway
  smoke, runner registry smoke, compose runner smoke, and SQS/LocalStack smoke
  when local prerequisites are available.

Required exercise scenarios:

- basic completed goal with evidence and satisfaction rationale;
- pending human prompt backed by a delayed compute thunk and continuation;
- approval request with approve and reject paths;
- blocked task with retry, replan, add-context, and cancel recovery actions;
- cancelled goal with queue cleanup and retained history;
- signal-driven goal from fake PR/CI/GitHub Actions/GitLab-style event input;
- fanout where child tasks become visible subgoals or workstreams;
- fork/join review where candidates, reviewer votes, selected branch, and
  unifier rationale are visible;
- memory/research evidence with citations, source artifacts, and information-use
  plan;
- long iterative loop with bounded retries, progress, budget, and stop reason;
- runner availability/routing scenario with registered runners and rejected
  runner reasons;
- event/notification scenario with inbound and outbound SQS/LocalStack messages.

Exercise output:

- write per-scenario artifacts under `target/coat-scenarios/<scenario-id>/`;
- write bootstrap/demo artifacts under `target/coat-scenarios/bootstrap/` and
  `target/coat-scenarios/live-bootstrap/`;
- write a rollup report at
  `target/coat-scenarios/latest/system-exercise.json`;
- include submitted goal IDs, selected-goal projection, graph snapshot, action
  queue snapshot, worker-run summaries, evidence refs, event refs, reset actions,
  command exit statuses, and links to SPA screenshots or TUI transcripts when
  available.

Acceptance for the exercise harness:

- `make reset-smoke`, `make bootstrap-scenarios`,
  `make validate-task-graph-bootstraps`, `make scenario-e2e`, and
  `make scenario-e2e-ui` remain the CI-facing pieces.
- `scripts/coat-exercise-system.sh --mode demo` creates useful state that is
  immediately navigable in both SPA and TUI.
- `scripts/coat-exercise-system.sh --mode quick` runs without Docker and catches
  broken scenario specs, reset drift, and shell-script regressions.
- `scripts/coat-exercise-system.sh --mode full` is the local pre-release
  confidence pass and records enough evidence to debug failures after the fact.

#### Phase 5: Live Runtime Proof

- Run the remaining live-runtime follow-ups only after the simplified operator
  surface is stable enough to inspect the results:
  Restate restart/resume, Codex App Server live worker, kind/k3d executor Jobs,
  Qdrant/Graphiti/Zep, S3/MinIO object snapshots, SQS/provider adapters, and
  release/Helm smoke.
- Every live proof must produce replay fixtures and visible operator evidence.

Exit criteria:

- live proofs are env-gated, replayable, and visible in SPA/TUI evidence and
  worker-run panels;
- no live path can satisfy a goal with placeholder output.

#### Phase 6: Closure And Deletion

- Delete superseded UI helpers, docs, tests, and scripts after replacement tests
  pass.
- Move this master plan to completed only after every remaining follow-up is
  satisfied, explicitly superseded, or moved into a new active plan with a
  narrower owner.

Exit criteria:

- `coat plan follow-ups` is empty or names only intentionally deferred live
  provider work;
- `cargo test --workspace`, `make ci-node`, `make scenario-e2e`, and
  `make scenario-e2e-ui` pass in CI-capable environments;
- local operator bootstrap produces navigable, useful demo state in SPA/TUI.

### Implementation Workstreams For The Next Execution Turn

When execution starts, use durable child-task workstreams with non-overlapping
write scopes:

- `SurfaceAuditor`: inventory and deletion plan across SPA/TUI/CLI/MCP/docs.
- `StateMachineCore`: actor transition matrix, domain module split, invariants,
  property/model tests, and projection rebuild tests.
- `BackendProjection`: goal-store/operator action/draft/timeline persistence.
- `GatewayStream`: compact operator API and SSE projection contract.
- `SPAWorkbench`: shadcn workbench and simplified chat/draft/action UX.
- `TUIWorkbench`: tab/focus/scroll/action UX aligned with SPA.
- `ScenarioUsability`: behavioral and visible-outcome scenario tests.
- `BootstrapDemo`: reset/bootstrap scripts and useful demo states.
- `DocsUnifier`: docs and active-plan follow-up cleanup.
- `ReleaseCI`: CI/runtime validation, Node/Rust cache, release smoke alignment.
- `Reviewer`: code-review, usability-review, and deletion-risk gate.

### Execution Decisions

Recorded 2026-05-15:

- SPA keeps multiple visible routes, but each route must be simplified around a
  clear operator task and the shared selected-goal context.
- Chat defaults to `Ask` when no goal is selected.
- TUI `Enter` activates the focused row or panel control. Chat submission uses a
  modified keybinding so navigation and chat do not fight each other.
- The first live-proof batch after simplification includes all three reference
  proofs: Restate restart/resume, Codex App Server, and kind/k3d executor Jobs.
- Old debug/helper endpoints and surfaces should be deleted immediately when a
  replacement operator path exists. Do not keep compatibility shims for removed
  surfaces.

### First-Wave Implementation Evidence

Recorded 2026-05-15:

- `StateMachineCore` added domain-level terminal mutation guards and
  table/property-style state-machine tests for stale terminal mutations,
  cancelled-goal immutability, terminal-task stale worker results, deterministic
  projections, and non-stub placeholder satisfaction rejection. Remaining work:
  carry those precise recovery errors through coordinator, gateway, and operator
  projection surfaces.
- `BackendProjection` added goal-store read projections for operator timeline,
  worker runs, and evidence, tightened draft/action resolution so accepting one
  draft does not resolve unrelated actions, and added focused projection tests.
  Remaining work: wire these projections fully into gateway/SPAs and add live
  Postgres-backed projection tests.
- `GatewayStream` made `/api/operator/actions/{action_id}/resolve` preserve
  upstream workflow status such as `409`, added recovery actions to validation
  failures, projected compute-graph thunks into `/api/operator/actions`, and
  tightened SSE filtering/reconnect metadata. Remaining work: replace polling
  stream internals with richer run/event projection only after backend event
  records are stable.
- `SPAWorkbench` simplified the React/Vite operator surface: chat defaults to
  Ask, goal drafting is explicit, drafts have clearer edit/discard/accept
  controls, labels moved toward product language, the dashboard surfaces recent
  runs, and action cards warn when required refs are missing. Remaining work:
  keep shrinking route panels around selected-goal outcomes and add browser E2E
  for visible operator behavior.
- `TUIWorkbench` made Enter activate focused rows/controls, moved chat submit
  to `Ctrl-S` and modified Enter where available, kept input editable while a
  request is pending, added scroll-state labels, exposed restart with `Alt-R`,
  and extended TUI tests for keyboard/restart behavior. Remaining work:
  manually smoke a real terminal session and add gateway/backend integration
  tests for action resolution.
- `ScenarioScripts` added `scripts/coat-exercise-system.sh` with quick, demo,
  e2e, ui, and full modes, Make targets, reset-smoke syntax validation, and a
  summary writer for `target/coat-scenarios/latest/system-exercise.json`.
  Remaining work: expand bootstrap/demo states beyond the current live and
  fixture coverage until completed, running, pending-action, approval, blocked,
  fanout, fork/join, signal, memory/research, and cancelled-history scenarios
  are all navigable in SPA and TUI.
- `MCP/operator` docs and smoke coverage now treat
  `coat_operator_workspace`, `coat_operator_goal`, `coat_operator_actions`,
  `coat_operator_action_resolve`, `coat_operator_agent_context`,
  `coat_operator_goal_submit`, and `coat_operator_goal_steer` as the compact
  operator surface. Old overview/snapshot/activity/approval/runner helper tool
  names and old HTTP helper routes are not compatibility targets.
- Live proofs are not complete yet. Restate restart/resume, live Codex App
  Server, kind/k3d executor Jobs, live memory adapters, and published release
  smoke remain follow-ups until this plan records direct evidence.

### DocsPlanUnifierReview Evidence

Recorded 2026-05-15:

- Reviewed the current documentation edits against the master plan, compact
  operator API docs, chat-client MCP guidance, TUI/CLI docs, local exercise
  wrapper docs, release smoke notes, and product spec updates. The docs now point
  operators to `/api/operator/*`, `/api/operator/stream`, the compact MCP
  operator tools, and the system exercise wrapper without reopening completed
  subsystem plans.
- `target/debug/coat plan follow-ups --json` reports one active plan and 24
  follow-ups for that review snapshot. No follow-up was removed in that review
  because the remaining items still require live proof, broader scenario
  evidence, provider credentials, terminal/browser artifact capture, or release
  evidence.
- Local documentation checks for this review: `make docs-check` passed, and
  `git diff --check` passed.

### PlanAndCIUnifier Evidence

Recorded 2026-05-15:

- Reviewed the current dirty diff without reverting unrelated work. The
  PlanAndCIUnifier slice checked only plan, CI, Make, and operator-doc wiring
  around `runtime-live-scaffold`, deterministic `scenario-e2e`, and UI smoke
  paths. Runtime live scaffold remains a readiness artifact only; it writes
  `live_proof_executed=false` and does not start Docker, Restate, Codex App
  Server, kind, k3d, kubectl, or Kubernetes workloads.
- CI/Make wiring currently runs `make runtime-live-scaffold` in the normal CI
  build and `ci`/`ci-pr` targets, keeps `make scenario-e2e` as the deterministic
  backend scenario path, keeps `make scenario-e2e-ui` as the fixture-backed
  Playwright path, and leaves `make scenario-e2e-ui-live` as the real local
  Compose gateway browser proof.
- Corrected local-dev docs that still referenced the old
  `scripts/coat-scenarios/*.json` scenario spec path; the actual checked-in
  scenario specs are under `scenarios/e2e/*.json`.
- `target/debug/coat plan follow-ups --json` reports one active plan and 24
  follow-ups for that review snapshot. No follow-up was removed in that review
  because the remaining items still require live proof, broader scenario
  evidence, provider credentials, terminal/browser artifact capture,
  token-broker design, or release evidence.
- Local validation for this slice passed: `make docs-check`,
  `target/debug/coat plan follow-ups --json`, and `git diff --check`.
- Final integration review 2026-05-15 closed the local deterministic
  state-machine, scenario-bootstrap, and Phase 0-3 simplification follow-ups.
  Remaining follow-ups are the live/provider/browser/cluster/release proofs and
  longer-running UI/module hardening items.

### Worker6 Plan Cleanup Evidence

Recorded 2026-05-15:

- Reviewed `target/debug/coat plan follow-ups --json` and this active plan. The
  current projection before this cleanup reported one active plan and 19
  follow-ups.
- Local deterministic evidence is recorded for state-machine guards and
  projection tests, no-stack scenario/bootstrap coverage, SPA/TUI simplification,
  gateway action recovery, and runtime-live-scaffold gating. Those items are
  closed as local deterministic proof, not as live Restate, live Codex, kind/k3d,
  live memory, provider, Slack, or release proof.
- The Restate, Codex App Server, kind/k3d, Qdrant/Graphiti/Zep, Slack/provider,
  token-broker, and release follow-ups remain open because this plan has no
  direct workspace evidence that those live proofs completed.
- This cleanup reran the requested local gates: `make docs-check` passed,
  `target/debug/coat plan follow-ups --json` reported one active plan and 18
  follow-ups, and `make runtime-live-scaffold` passed with Restate
  restart/resume, Codex App Server, and kind/k3d executor proofs skipped
  because their explicit live gates were not set.
- No live follow-up was closed in this cleanup. The runtime scaffold output is
  readiness evidence only; it records `live_proof_executed=false` and does not
  contact Docker, Restate, Codex App Server, kind, k3d, kubectl, Kubernetes, or
  model/provider services.

### Six-Worker Simplification Evidence

Recorded 2026-05-15:

- SPA extraction continued without changing the product model: graph/goal
  presentation and projection helpers moved into
  `ui/control-plane-web/src/spa/features/goal-graph-panel.tsx`, while
  action/evidence/queue controls moved into
  `ui/control-plane-web/src/spa/features/operator-action-panels.tsx`. The
  remaining App shell still owns route/state composition, selected-goal context,
  and backend API calls.
- Gateway operator projections now attach product-shaped recovery data with
  concrete `actions` and `suggested_resolutions` to stale or failed action
  outcomes. Smoke coverage asserts `/api/operator/workspace`,
  `/api/operator/goals/:id/graph`, `/api/operator/actions`, and
  `/api/operator/stream` in operator terms rather than raw Restate error text.
- TUI action resolution now keeps the last action result visible in the Actions
  panel, including recovery hints. `Ctrl-L` clears this local result along with
  local chat/action state.
- Scenario runs now write deterministic operator evidence under each run's
  `operator/` directory: selected goal, action queue, graph nodes, evidence,
  worker runs, chat/draft state, normalized snapshot, and `transcript.md`.
  `report.json` links those artifacts so failures can show what an operator
  would have seen without requiring a live browser.
- Local validation for this slice passed: `npm run --prefix
  ui/control-plane-web build`, `npm run --prefix ui/control-plane-web smoke`,
  `cargo test -p coat-cli tui`, `cargo test -p coat-cli scenario`,
  `cargo test --workspace`, `make scenario-e2e stack=never
  SCENARIO_E2E_KEEP_STACK=0`, `make bootstrap-scenarios`, `make reset-smoke`,
  `make runtime-live-scaffold`, `make docs-check`, `cargo fmt --all --check`,
  and `git diff --check`.
- No live-provider or cluster follow-up was closed. The remaining live
  follow-ups still require the explicit gates listed below.

## Workstreams

### Runtime Proof

- Evidence 2026-05-18: `crates/coordinator/tests/restate_restart_resume.rs`
  now contains a real Docker-backed live RuntimeVerifier harness. With
  `COAT_RESTATE_RESTART_RESUME_TEST=1`, Docker daemon access, a pinned
  `COAT_RESTATE_TESTCONTAINERS_IMAGE`, and `target/debug/coat-coordinator`, the
  ignored test starts Restate with persistent data, starts the coordinator on a
  dynamic port, registers the deployment, submits a goal through Restate
  ingress, restarts the coordinator, restarts Restate against the same data
  directory, and compares the durable goal state across all boundaries.
- Evidence 2026-05-18: live proof command passed locally:
  `COAT_RESTATE_RESTART_RESUME_TEST=1
  COAT_RESTATE_TESTCONTAINERS_IMAGE=docker.restate.dev/restatedev/restate:1.5
  cargo test -p coat-coordinator restate_restart_resume_proof_entrypoint --
  --ignored --exact --nocapture`.
- Evidence 2026-05-18: `scripts/coat-runtime-live-scaffold.sh` now runs the
  Restate restart/resume proof when `COAT_RESTATE_RESTART_RESUME_TEST=1` is
  enabled and records `live_proof_executed=true` in its JSON summary. Local
  proof passed with output under
  `target/coat-runtime-live-scaffold-live-smoke/runtime-live-scaffold.json`;
  the default `make runtime-live-scaffold` path still skips live work unless the
  explicit gate is enabled.
- The ignored `coat-coordinator` RuntimeVerifier entrypoint remains safe for
  normal CI: default tests compile the gate/config/idempotency path without
  touching Docker, and the live proof requires explicit opt-in env vars.
- Evidence 2026-05-11: `crates/coordinator/tests/restate_restart_resume.rs` now has deterministic config, harness-step ordering, projection idempotency, and transition-counter assertions around the live proof gate.
- Evidence 2026-05-11: coordinator transition observations now include waiting-input counts, pending delayed thunks, mechanism-round counts, compute-graph node/edge counts, and a `coordinator.transition` tracing span; RuntimeVerifier projection counters now assert persisted compute-graph nodes, edges, open thunks, and waiting tasks.
- Evidence 2026-05-14: coordinator control handlers now route through a shared serialized transition path for cancel, feedback, steer, approve, restart, branch, select-branch, vote, delayed thunk, and mechanism actions. Tests prove blocked, waiting, and failed goals can recover through restart or resume while done and cancelled goals stay closed.
- Evidence 2026-05-15: `scripts/coat-runtime-live-scaffold.sh` and `make runtime-live-scaffold` record Restate restart/resume readiness as skipped by default, failed for unsafe config, or blocked when gates are missing. The earlier unimplemented-harness blocker was superseded by the 2026-05-18 live proof.
- Evidence 2026-05-15: coordinator `create_thunk` replay handling now treats
  exact duplicate task/continuation pairs as idempotent while preserving the
  domain invariant that rejects reusing the same continuation for a different
  task. Focused coordinator coverage proves the conflict path.
- Remaining runtime proof work is observability depth: export and assert spans
  once an OpenTelemetry sink endpoint is selected for the live harness.
- Assert completed durable steps are not re-executed after replay.
- Add transition metrics and spans for workflow run, task dispatch, runner calls, validation, restart, approval pause/resume, projection attempts, and projection failures.
- Populate trace IDs already present in worker/protocol metadata instead of creating a separate observability model.

### Codex Live Worker

- Make distributed/operator profiles truthfully distinguish stub execution from live execution.
- Prevent placeholder actor, critic, unifier, or research results from satisfying non-stub goals.
- Implement Codex App Server `/run-task` as the first live runner path.
- Evidence 2026-05-11: `sidecars/codex-runner-ts` now has explicit `stub`, `replay`, `live`, and `mcp-healthcheck` modes; live App Server mode blocks rather than fabricating work when auth, URL, sandbox, or workspace gates are missing; replay mode consumes `examples/codex-app-server-replay.json`.
- Preserve sandbox profile, memory context refs, thread/session IDs, child-task requests, git refs, checkpoints, object refs, and artifact manifests in `AgentRunResult`.
- Add Codex MCP as the fallback callable-tool path after App Server behavior is proven.
- Evidence 2026-05-11: `sidecars/codex-runner-ts` now has `mcp-replay` mode and `examples/codex-mcp-fallback-replay.json`, so Codex MCP fallback parsing, diagnostics, and structured result extraction are covered by deterministic CI tests.
- Capture replay fixtures with thread IDs, checkpoint refs, git refs, artifact manifests, structured results, and diagnostics.
- Verify live provider profiles after auth setup is exercised on real nodes, including Codex App Server, Claude Code, Bedrock, vLLM, Ollama, Hugging Face, and OpenAI-compatible gateways.
- Evidence 2026-05-11: `/verify` now returns provider-profile entries with explicit `verified`, `skipped`, or `failed` state so unavailable live provider routes produce reviewable skipped evidence instead of silent absence.
- Evidence 2026-05-15: the runtime live scaffold added a separate
  `COAT_CODEX_APP_SERVER_LIVE_PROOF` gate that requires live mode, App Server
  auth, endpoint URL, and an existing isolated workspace before a Codex App
  Server smoke can be attempted.
- Evidence 2026-05-18: the runtime live scaffold now runs the Codex App Server
  proof when `COAT_CODEX_APP_SERVER_LIVE_PROOF=1` is enabled. It builds
  `sidecars/codex-runner-ts`, calls `/verify`, runs the typed `/run-task`
  contract through `sidecars/codex-runner-ts/scripts/codex-app-server-live-proof.mjs`,
  records `verify.json`, `run-task-result.json`, `summary.json`, and
  `live-proof.log`, and marks the proof as failed if the runner returns
  anything other than a structured `status=done` result.
- Evidence 2026-05-18: live Codex App Server `/run-task` passed against
  `codex-cli 0.130.0` with `codex app-server --listen
  ws://127.0.0.1:17890`, `CODEX_RUNNER_MODE=live`,
  `CODEX_AUTH_MODE=app_server`, and an isolated
  `target/codex-live/workspace`. The runner returned `status=done`,
  structured command evidence, a metadata checkpoint, sandbox declaration, and
  the live transcript refs `thread=019e3caf-5238-7813-a0da-067a2c54bee6` and
  `turn=019e3caf-52a1-79e0-8907-0ab4d8b5e7cf`. Local evidence was captured at
  `target/codex-live/run-task-result.json`; the committed replay fixture
  remains the sanitized deterministic fixture under
  `examples/codex-app-server-replay.json`.
- Evidence 2026-05-18: the first live attempt exposed a current App Server /
  Responses structured-output requirement: all object schemas must set
  `additionalProperties: false`. `sidecars/codex-runner-ts` now emits a strict
  `AgentRunResult` output schema and has a regression test that walks the schema
  to reject non-strict object nodes before another live run hits that API error.
- Evidence 2026-05-18: live provider verification passed for the configured
  Codex App Server and Codex MCP fallback routes with `CODEX_VERIFY_APP_SERVER=1`,
  `CODEX_VERIFY_MCP=1`, and `CODEX_VERIFY_PROVIDER_NETWORK=1`. The captured
  profile evidence in `target/codex-live/verify-app-server-and-mcp.json` reports
  both `codex:app_server` and `codex:mcp` as `verified` and confirms no secret
  values were exposed.
- Keep staff-engineer live execution second-phase until current `@ctxr/kit` and `@ctxr/agent-staff-engineer` behavior, isolated target repo install, tracker auth, and Claude Code auth distribution are verified.
- Add a live staff-engineer issue-to-PR smoke test only after those staff-engineer gates pass.

### Executor Provisioning

- Connect coordinator-approved runnable task state and capacity policy to sandbox-runner Kubernetes executor Job provisioning. The coordinator must approve budget, approval policy, sandbox profile, local tools, `ExecutionProfile.capacity`, and template refs before any Job is materialized.
- Use kind or k3d in CI to prove the normal backend path: coordinator capacity approval, sandbox-runner provision request, Kubernetes server-side dry-run, live apply, Job/Pod watch, result ingestion, and goal-store projection.
- Implement the `jattg-agent-toolbox` executor contract: read `sandbox-launch-plan.json`, run bounded work, write command/artifact/checkpoint/git/object-store manifests, emit the structured result, and produce sandbox attestation evidence.
- Watch Job and Pod state, collect logs and applied/final manifests, classify image-pull, scheduling, runtime-class, admission, timeout, deadline, and cleanup failures, enforce TTL/cleanup, and project results into goal-store.
- Preserve provision request ID, goal/task IDs, capacity decision ref, ConfigMap/Job/Pod UIDs, phase transition timestamps, log refs, result manifest refs, and attestation evidence as validator-reviewable artifacts.
- Evidence 2026-05-11: `crates/sandbox-runner` now mounts `provisioner-evidence.json`, injects `COAT_*` evidence paths into executor Jobs, and rejects live modes before cluster contact when coordinator evidence refs are absent.
- Evidence 2026-05-15: the runtime live scaffold adds a `COAT_KUBERNETES_EXECUTOR_LIVE_PROOF` readiness gate for the kind/k3d proof. It requires `SANDBOX_ENABLE_KUBERNETES_PROVISIONER=true`, `kubectl`, kind or k3d, a `server_dry_run` or `apply` proof mode, and coordinator capacity/template/result-ingestion refs, but it does not contact the Kubernetes API.
- Keep rendered Job manifests as operator fixtures and escape hatches; the normal backend path uses Rust `kube`/`k8s-openapi` clients.

### Memory, Research, And Object Artifacts

- Run live Qdrant and Graphiti/Zep tests only when explicit service URLs and credentials are present.
- Keep replay fixtures in normal CI so research provenance can be reviewed without live web or model access.
- Route live research through a bounded runner/tool path that requires citations, source artifacts, and an `InformationUsePlan`.
- Capture raw source snapshots, fetch metadata, and large artifacts into MinIO/S3-compatible storage.
- Promote local object refs to real uploaded snapshots once the uploader path is live.
- Preserve memory context by reference; do not dump large memory payloads into task prompts.
- Evidence 2026-05-11: `coat-memory-gateway` now preserves research source/object artifact refs in Qdrant payloads and Graphiti source descriptions, and `examples/research-output-memory-substrate.json` carries raw source snapshot and fetch metadata object refs for replay review.
- Evidence 2026-05-11: live Qdrant, Graphiti, and Zep tests are explicitly env-gated while replay fixture coverage remains deterministic in normal CI.

### Events And Notifications

- Add SQS LocalStack inbound event-source and outbound notification smoke tests. First proof slice: `make eventops-sqs-smoke` starts LocalStack when Docker is available, reuses the SQS event-source and notification examples with local queue URLs, proves inbound poll/delete through `coat-event-gateway`, and proves outbound SQS delivery through `coat-notifier`.
- Evidence 2026-05-11: `coat-notifier` now has a journaled outbox with `pending`, `delivered`, `awaiting_ack`, `acknowledged`, `retry_scheduled`, and `dead_lettered` states plus `/outbox`, `/outbox/{id}/ack`, `/outbox/{id}/retry`, `/outbox/retry-due`, and `/dlq` endpoints.
- Evidence 2026-05-11: `make eventops-sqs-smoke` passed against `localstack/localstack:3.8.1`, proving inbound SQS poll/delete, outbound SQS delivery, notifier journal replay shape, and an `awaiting_ack` outbox entry.
- Evidence 2026-05-18: `make eventops-sqs-smoke` passed locally with Docker
  access, rebuilding `coat-event-gateway`, `coat-goal-store`, and
  `coat-notifier`, starting LocalStack SQS, registering the inbound event
  source, polling and deleting the inbound SQS event through event-gateway, and
  delivering an outbound notification envelope through notifier.
- Evidence 2026-05-11: event sources now include explicit `pull_request_check`, `github_actions_check`, and `gitlab_pipeline_check` kinds with examples and normalization tests, so PR required checks, GitHub Actions runs, and GitLab pipelines all project provider-neutral `_coat_change_activity` metadata before routing durable goals.
- Closure 2026-05-11: the SQS/LocalStack residuals inherited from the distributed-runners and events plans are satisfied by the smoke above; remaining EventOps work is topology proof plus additional provider adapters.
- Normalize recurrent observability events into durable gateway events before creating or steering goals.
- Keep event activation behind coordinator policy and human approval when sources add external callbacks, cost-bearing polling, or broad network access.
- Prove event-gateway projection against the same Compose or cluster topology operators run, not only isolated local smoke scripts.
- Evidence 2026-05-11: `coat-event-gateway` now projects create-goal trigger decisions with concrete `goal_id` values into `coat-goal-store`; `make event-gateway-smoke` verifies source registration, normalized event ingestion, trigger creation, goal-store projection, and dedupe behavior.
- Evidence 2026-05-12: `make event-gateway-compose-smoke` now runs the same
  proof against the deterministic Compose topology operators use: it starts the
  stack, registers an approved create-goal CI source, emits and dedupes a
  synthetic CI failure, verifies Restate submission and goal-store projection,
  records evidence under `target/coat-event-gateway-compose-smoke/latest`, and
  tears the stack down.
- Add Slack, PagerDuty, Google Calendar, Outlook, and OpenTelemetry provider adapters after the SQS proof is stable and credentials are approved.

### Operator UI And MCP

- Add browser E2E over the full Compose stack for goal selection, task graph inspection, memory preview/apply, approval/reject/comment, runner status, and event source management.
- Evidence 2026-05-11: `ui/control-plane-web` smoke coverage now renders event-source activation, approval queues, runner capacity, memory events, goal progress, and task graph filters against gateway-backed fixtures.
- Evidence 2026-05-11: gateway goal detail reads `GoalWorkflow/compute_graph`,
  exposes `workflow_compute_graph` through `/api/operator/goals/{goal_id}` and
  MCP `coat_operator_goal`, and the SPA renders `waiting_input` continuation
  state plus compute-graph node/edge/thunk counters in task summaries.
- Evidence 2026-05-11: operator continuations are now actionable in the SPA: open delayed-compute thunk nodes render reason, task ID, thunk ID, continuation ID, wait ref, response-summary input, and a guarded `resume_thunk` backend mutation; resumed/cancelled thunks are filtered out of the actionable queue.
- Verify UI mutations use backend APIs only and never mutate goal-store projections directly.
- Keep existing gateway contract smoke tests as the fast CI path.
- Evidence 2026-05-14: PLAN-1 backend-first operator projection started by adding
  `/api/operator/*` as the SPA/TUI-facing surface, routing goal submit and
  action resolution through typed backend APIs, appending durable operator event
  envelopes to `coat-goal-store`, and streaming `/api/operator/stream` with
  product-level event names plus operator-event filtering.
- Evidence 2026-05-14: the React/Vite SPA now uses real Tailwind/shadcn
  dependencies for the first operator workspace card/components while keeping
  the current app architecture and backend-owned chat/draft persistence.
- Evidence 2026-05-14: the Rust TUI now consumes `/api/operator/workspace` and
  resolves actions through `/api/operator/actions/{action_id}/resolve`, so
  approval/thunk/recovery commands share the same operator projection as the
  SPA.
- Evidence 2026-05-14: the SPA Action Queue now reads
  `/api/operator/actions`, resolves human prompts through
  `/api/operator/actions/{action_id}/resolve`, and treats product-level SSE
  events such as `goal.updated`, `task.updated`, `approval.requested`, and
  `goal.cancelled` as workspace projection updates.
- Evidence 2026-05-15: `/api/operator/stream` now emits state-derived
  product-level events including `action.required`, `approval.requested`,
  `goal.satisfied`, `goal.cancelled`, and `stream.heartbeat`; the SPA applies
  each streamed workspace projection to the global goal list, selected-goal
  detail, workspace, and action queue caches so visible state does not wait for
  the next poll. Projection fallback IDs are now derived from stable row content
  instead of random UUIDs, allowing unchanged stream reads to emit heartbeats.
  The control-web smoke test reads real SSE blocks and asserts event name,
  heartbeat behavior, and selected-goal projection.
- Evidence 2026-05-14: the old browser/operator helper routes for overview,
  runner lists, human threads, and plan follow-up queues were removed from the
  SPA/gateway public surface. SPA, TUI, scenario, and smoke coverage now use
  `/api/operator/workspace`, `/api/operator/goals`, and
  `/api/operator/actions` for product state; durable plan continuity remains
  under `/api/plans/{plan_id}/continuity` and MCP `coat_plan_continuity`.
- Evidence 2026-05-14: MCP read and mutation tools now mirror the operator
  state-machine surface with `coat_operator_workspace`, `coat_operator_goal`,
  `coat_operator_actions`, `coat_operator_action_resolve`,
  `coat_operator_agent_context`, `coat_operator_goal_submit`, and
  `coat_operator_goal_steer`; old overview/snapshot/activity/approval/runner
  compatibility tool names were removed from docs, skill guidance, and smoke
  fixtures.
- Evidence 2026-05-14: the TUI no longer needs a separate `/api/config`
  startup read. The sanitized gateway/chat configuration summary is carried in
  `/api/operator/workspace`, the standalone config route was removed, and the
  remaining gateway composition helper is named as a backend projection instead
  of the old overview surface.
- Evidence 2026-05-14: the SPA module cleanup continued by extracting the
  selected-goal runtime bar, active-draft dock, and generic operator primitives
  from `App.tsx` into typed React feature/component modules. The extracted
  components remain presentation-only over the `/api/operator/*` projection and
  do not own durable state.
- Evidence 2026-05-14: the SPA cleanup continued by simplifying chat starter copy,
  hiding raw session IDs behind workspace or selected-goal labels, adding active
  draft edit/discard/submit controls, and exposing direct approval rejection from
  approval prompts while keeping draft submission routed through the operator API.
- Evidence 2026-05-14: the TUI now has operator tabs for Overview, Goals, Graph,
  Actions, Approvals, Events, Workers, Evidence, Adversarial, and Debug. Actions,
  approvals, graph, worker runs, and evidence all render from operator projections,
  and the old command-coverage wording was removed from the TUI debug surface.
- Evidence 2026-05-14: reset and bootstrap helpers now use the same bootstrap
  evidence root as the Makefile (`target/coat-scenarios/bootstrap`), bootstrap
  cleanup removes checked-in scenario IDs from generated evidence roots instead
  of broad directories, and `scripts/coat-bootstrap-scenarios.sh`,
  `make bootstrap-scenarios`, `make bootstrap-reset`, `make scenario-reset`,
  and `make compose-reset-dry-run` were validated. CI now also runs
  `make reset-smoke` so reset help, shell syntax, scenario cleanup, bootstrap
  cleanup, and Compose dry-run cleanup remain covered.
- Evidence 2026-05-15: `make bootstrap-goals` now submits fixed demo goals
  through `coat goal submit` and creates a durable human-input thunk through
  `coat goal thunk create`, leaving one satisfied executor lifecycle goal, one
  pending approval goal, and one pending human prompt goal visible in the
  SPA/TUI. Direct read-model fixture projection remains explicit as
  `make bootstrap-fixture-goals` or `coat scenario seed`.
- Evidence 2026-05-15: deterministic bootstrap scenarios now include completed,
  running, pending-action, approval, blocked/recovery, fanout, fork/join,
  signal-driven, memory/research, cancelled-history, and operator-usability
  fixtures. `make bootstrap-scenarios` and `make scenario-e2e stack=never`
  passed over the full fixture set, including `bootstrap_running`,
  `bootstrap_pending_action`, and `operator_usability_workbench`.
- Evidence 2026-05-12: CI and docs now define the deterministic PR-gated
  scenario workstream as a loop over `scenarios/e2e/*.json` with
  `target/debug/coat scenario run --file <scenario> --output-dir
  target/coat-scenarios`; failure artifacts include `target/coat-scenarios`
  plus control-web Playwright traces, screenshots, `test-results`, and reports.
  The scenario policy requires stubbed workers, fixed seeds, bounded clocks,
  backend API mutations, and current-goal selector evidence across SPA and TUI
  surfaces.
- Evidence 2026-05-12: `make scenario-e2e-ui-live` now starts the
  deterministic Compose stack, drives a chat-authored goal through the real
  control gateway, selects the returned goal, and waits until goal-store
  projection appears in the goal list and selected-goal work graph.
- Evidence 2026-05-12: the same live-stack browser proof now exercises
  backend-routed memory write, memory preview/apply, human queue visibility,
  registered runner status, and event-source registration through the real
  gateway before shutting the deterministic stack back down.
- Evidence 2026-05-18: `make scenario-e2e-ui` passed locally after the
  simplified SPA assertions were aligned with the current draft/action wording
  and selected-goal model. The fast browser suite reported six passing specs
  with the live-stack spec intentionally skipped outside the live gate.
- Evidence 2026-05-18: `make scenario-e2e-ui-live` passed locally with Docker
  access. It built the React/Vite control web app, started the deterministic
  Compose stack with stub runners, drove a chat-authored draft through accept
  and submit, observed the goal-store projection, verified the selected goal in
  goals and work graph views, exercised memory write plus preview/apply,
  confirmed the Action Queue and feedback-thread surfaces, verified registered
  runner rows with node/capacity/endpoints, registered an event source, and
  tore the stack down cleanly.
- Evidence 2026-05-15: the SPA chat/draft surface was split into
  `ChatDraftPanel`, Ask is the default chat mode, Draft goal is explicit, and
  active draft controls use direct edit/discard/accept wording without showing
  raw `operator:default` session IDs. The browser live-stack spec now uses the
  current Action Queue naming.
- Evidence 2026-05-15: goal-scoped gateway workspace/action projections now
  request filtered goal-store paths and defensively filter actions, events,
  worker runs, evidence, approvals, and tasks by `goal_id`; invalid action
  resolutions return `400` with recovery guidance instead of falling through to
  restart. Control-web smoke covers cross-goal filtering and invalid-action
  recovery.
- Evidence 2026-05-15: SPA module cleanup continued by moving action-needed
  projection, continuation queue rendering, and work-graph rendering into
  feature modules (`operator-action-panels.tsx` and `task-graph-view.tsx`) while
  keeping `App.tsx` as the route shell. Validation passed with
  `npm run --prefix ui/control-plane-web build` and
  `npm run --prefix ui/control-plane-web smoke`.
- Evidence 2026-05-15: the operator action configuration surface moved into
  `operator-control-panel.tsx`, including voting, steering, restart/branch,
  wait-state, mechanism-round, review, and research payload builders. The SPA
  shell now imports the feature route instead of owning those action builders;
  build, smoke, scenario, and TUI focused tests passed.
- Evidence 2026-05-15: the memory route moved into `memory-view.tsx`, including
  memory search/context/write, replacement preview/apply, diff rendering, and
  memory event projection. The memory render smoke now loads the feature module
  directly, and the SPA build and smoke suite passed after the extraction.
- Evidence 2026-05-15: dashboard, plan, action-queue, event-source, worker-run,
  service-strip, and runner-fleet route surfaces moved into
  `operator-dashboard-routes.tsx`. `App.tsx` is now closer to a route shell and
  shared chat/selection state owner, and the smoke guardrail reads the extracted
  module when proving `/api/operator/actions` usage.
- Evidence 2026-05-15: operator SSE connection, event parsing, reconnect
  behavior, and TanStack Query projection-cache updates moved into
  `operator-stream.ts`; `App.tsx` consumes the hook instead of owning stream
  protocol details. SPA build and smoke passed after the extraction.
- Evidence 2026-05-15: the TUI action and approval surfaces now support
  approval rejection with `r`, including optional rejection text, while keeping
  selected-goal scoping and modified-key chat submission. `cargo test -p
  coat-cli tui` passed with the new rejection coverage.
- Closure 2026-05-18: the earlier Playwright rerun limitation is superseded by
  the passing fast and live browser E2E proofs above.
- Add token-broker-backed multi-user MCP smoke only after a broker implementation is selected.

### Release And Deployment Proof

- Fix release workflow command drift whenever CLI hierarchy changes.
- Evidence 2026-05-11: `.github/workflows/release-helm.yml` now uses `coat deploy chart package` for chart release packaging.
- Run published binary smoke and Helm chart smoke after the first GitHub Release and record evidence in `docs/operations/releases.md`.
- Evidence 2026-05-11: `.github/workflows/release-binaries.yml` and `.github/workflows/release-helm.yml` now include published-asset smoke jobs; `Makefile` exposes `release-binary-smoke` and `release-helm-smoke` for local/operator replay of those checks.
- Evidence 2026-05-11: CI, release-binary, and release-Helm workflows now use stable Rust cache keys plus `sccache`; GHCR publishing uses both GitHub Actions BuildKit cache and registry-backed `jattg-build-cache` refs; Rust and Node Dockerfiles use BuildKit cache mounts for target and npm caches.
- Evidence 2026-05-11: release binaries now build Linux ARM on the native `ubuntu-22.04-arm` runner and macOS ARM on `macos-latest`; CI adds a cached runner-target compatibility job matrix for `ubuntu-latest`, `ubuntu-24.04`, `ubuntu-22.04`, `ubuntu-24.04-arm`, `ubuntu-22.04-arm`, and `macos-latest`.
- Evidence 2026-05-11: GHCR image publishing can now target a single image or image group; the release workflow keeps Rust service/toolbox publishing in one cache-sharing job and fans Node sidecar images out into parallel jobs.
- Evidence 2026-05-11: Rust service release tags now share one multi-binary service image build; `COAT_SERVICE_BIN` selects the process, and the entrypoint preserves compatibility by mapping known `BIND_ADDR` ports to service binaries.
- Evidence 2026-05-12: CI now runs `compose-topology-smokes` on pull requests
  rather than on a cron schedule. The job runs `make compose-runner-smoke` and
  `make event-gateway-compose-smoke`, keeps workflow-dispatch escape hatches
  for targeted operator reruns, uses the same Rust cache/sccache setup as the
  rest of CI, and uploads Compose topology evidence on failure.
- Evidence 2026-05-18: published v0.0.3 binary smoke passed for
  `jattg-binaries-0.0.3-aarch64-unknown-linux-gnu.tar.gz` using the public
  GitHub Release URL; checksum verification, archive extraction, manifest
  parse, executable checks, `coat --help`, `coat guide --print`, and
  `coat release plan --version 0.0.3` all passed.
- Evidence 2026-05-18: published chart-v0.0.3 smoke passed using a pinned
  Helm v3.19.5 arm64 binary and the public
  `jattg-0.0.3.tgz` GitHub Release URL; checksum verification, chart lint, and
  template rendering passed. Cluster upgrade dry-run remains opt-in through
  `HELM_SMOKE_UPGRADE_DRY_RUN=true` on a cluster-capable runner.
- Evidence 2026-05-18: `release-helm-smoke` now uses `set -eu`, supports
  `HELM=/path/to/helm`, and no longer reports success after failed Helm
  commands.
- Add provider overlays after the first target is chosen; the first executor proof remains kind/k3d.
- Add Restate Cloud journal encryption guidance when the Rust service path and provider documentation support it.

### Protocol And SDK

- Keep protobuf, JSON schemas, docs, and Rust domain contracts aligned through `make proto-check`.
- Add generated Rust and TypeScript SDKs from Buf only after package names, output locations, and compatibility rules are selected.
- Evidence 2026-05-11: Buf SDK generation is scaffolded through `buf.gen.yaml`, `make proto-sdk-generate`, and `make proto-sdk-check`, generating Rust and TypeScript outputs under `target/generated-sdks` without committing generated artifacts.
- Decision 2026-05-11: generated SDK wrappers are internal validation artifacts for now, not published packages. Reserve `coat-protocol-sdk` and `@coat/protocol-sdk`, keep generation under `target/generated-sdks/`, and defer package metadata/release jobs until a published-SDK compatibility milestone is selected.
- Add or change public types only where needed for Kubernetes provision/result records, executor attestations, object upload status, retryable event delivery, and observability correlation.
- Evidence 2026-05-14: `crates/domain` now has shared operator projection and
  actor contracts for workspace snapshots, goal summaries/details, graph,
  actions, events, evidence, worker runs, durable event envelopes, and
  append/list operator-event API payloads.
- Evidence 2026-05-14: the operator actor contract validates goal, task, thunk,
  worker-run, review, approval, and append-only event transitions with recovery
  hints, preserving recoverable blocked/waiting/failed states while rejecting
  invalid transition attempts before they become UI no-ops.
- Evidence 2026-05-14: actor contracts now classify actor states as active,
  waiting, recoverable, terminal, or immutable; transitions declare the actor
  kinds they target, and invalid actor-kind transitions are rejected with
  recovery guidance before they can corrupt projections.
- Evidence 2026-05-11: `crates/domain` now models goal ranking votes as an opt-in extension with upvote/downvote promotion or demotion decisions, plus first-class delayed compute thunks for human input, approvals, timers, callbacks, resource waits, model availability, delimited continuation refs, worker `waiting` results, and derived compute graph snapshots; `coat-coordinator` exposes `vote`, `create_thunk`, `resume_thunk`, and `compute_graph`, and the CLI exposes `coat goal vote`, `coat goal thunk create`, `coat goal compute-graph`, and `coat human resume-thunk`.
- Evidence 2026-05-11: `crates/domain` now includes opt-in `mechanism_policy` and `MechanismRound` state for distributed consensus, voting, Delphi-style rounds, sealed-bid/Vickrey auctions, and contract-net allocation; `coat-coordinator` exposes `mechanism_start` and `mechanism_ballot`, and the CLI exposes `coat goal mechanism start|ballot`.
- Before moving any linked plan to completed, preserve every remaining follow-up here, record direct evidence, or write an explicit supersession note.

### Reviewer And Satisfaction Gates

- Evidence 2026-05-11: `examples/reviewer-fixtures/live-replay-worker-fixture.json` now consumes accepted Codex replay output as a reviewer/validator fixture with command evidence, child tester requests, git refs, worktree paths, and reviewer checkpoint branch evidence.
- Evidence 2026-05-11: `crates/domain` now rejects metadata-only checkpoints for code tasks that require git checkpointing, and the behavioral fixture proves actor output, reviewer output, tester child requests, and checkpoint branch projection into the goal-store snapshot.

## Tests

- State-machine tests cover the actor transition matrix for goals, tasks,
  thunks, worker runs, reviews, approvals, events, drafts, runners, memory, and
  mechanism rounds, including valid, invalid, stale, idempotent, recoverable,
  and terminal paths.
- Model-based/property tests generate bounded transition sequences and assert
  global invariants: terminal states, recoverability, continuation refs,
  idempotency, projection determinism, evidence provenance, branch validation,
  event dedupe, and non-stub satisfaction gates.
- Projection rebuild tests replay append-only domain events into
  `OperatorWorkspaceSnapshot`, action queue, actor graph, timeline, evidence,
  and worker-run projections without relying on gateway-local normalization.
- Unit tests cover task lifecycle, restart policy, projection idempotency, attestation validation, event retry states, object artifact refs, stub-output rejection, and public-contract serialization.
- Unit tests cover opt-in goal ranking vote promotion/demotion, delayed compute thunk pause/resume behavior, worker waiting-result thunk materialization, and compute graph projection of tasks, thunks, wait refs, and continuations.
- Unit tests cover opt-in mechanism rounds for consensus tallies and Vickrey auction decisions with human-ratification state.
- Restate integration tests cover coordinator restart, Restate journal recovery, durable projection replay, approval pause/resume, and timeout restart.
- Worker tests cover Codex App Server live execution behind env gates, Codex MCP replay, structured result extraction, and stub mode as an explicit smoke path only.
- Reviewer tests consume accepted live worker outputs as real-world fixtures and include git checkpoint branch/worktree coverage where branch workflows are involved.
- Kubernetes tests cover kind/k3d Job provision, execution, log/result collection, attestation, timeout, image-pull failure, runtime-class failure, scheduling failure, and artifact upload failure.
- Memory and research tests cover env-gated Qdrant/Graphiti/Zep, offline replay fixtures, source snapshots, citation validation, and memory repair/replay after adapter outage.
- Event and notification tests cover LocalStack SQS ingest, outbound notification, retry, ack, DLQ, event dedupe, and triggered-goal projection.
- UI tests cover existing gateway smoke plus deterministic `coat scenario run`
  browser E2E specs for full Compose operator workflows. The PR gate uploads
  scenario evidence and Playwright traces or screenshots on failure.
- Release tests cover binary unpack/checksum/operator-surface smoke and Helm template/install/rollout/rollback smoke after publication.

## Follow-Ups

2026-05-18 operator workflow review reopened one active internal follow-up:

- [ ] Durable plan/draft manager contract:
  - Persist chat-created plan drafts through the existing `/api/plans` /
    goal-store plan surface instead of keeping plan drafts as transient chat
    payloads.
  - Persist goal drafts through goal-store-backed draft records rather than the
    control gateway process-local map, so accept/discard works after restart
    and across nodes.
  - Make the primary authoring flow `Ask -> Plan -> Draft goal -> Accept draft
    -> Goal selected -> Work graph -> Actions/Evidence -> Satisfied`.
  - Surface selected-goal draft scopes explicitly: new goal, plan for selected
    goal, add subgoal, steer selected goal, or research request.
  - Add typed operator action resolution payloads for accept/discard/revise,
    and make draft refs match exactly when resolving action-queue rows.
  - Add scenario and browser/TUI tests for plan-first drafting, selected-goal
    draft scope, accept/edit/discard/submit, and queue updates after action
    resolution.
  - Completion criteria: SPA, TUI, MCP, and scenario runner can all manage the
    same durable draft/plan records without raw JSON or in-memory-only draft
    recovery.

## Deferred Or Deprecated Follow-Ups

- OpenTelemetry exported span assertions are deferred until an OTLP sink
  endpoint and assertion target are selected. Local probe on 2026-05-18 found
  no `COAT_OTEL_EXPORTER_OTLP_ENDPOINT` / `OTEL_EXPORTER_OTLP_ENDPOINT` and the
  Rust workspace currently only has the Compose collector fixture, not a
  selected exporter/assertion path.
- Staff-engineer live issue-to-PR smoke is deferred until `claude`,
  `@ctxr/kit`, `@ctxr/agent-staff-engineer`, tracker credentials, and auth
  distribution are available on an approved runner. Local probe on 2026-05-18
  found `claude` unavailable.
- Kubernetes executor Job proof is deferred until `kubectl` plus kind or k3d
  are installed and the coordinator evidence refs are selected. Local probe on
  2026-05-18 found `kubectl`, `kind`, and `k3d` unavailable; the existing
  runtime scaffold still records readiness/failure once
  `COAT_KUBERNETES_EXECUTOR_LIVE_PROOF=1` is set.
- Live Qdrant, Graphiti/Zep, and MinIO/S3-compatible adapter smokes are
  deferred until service URLs, credentials or brokered auth, embedding route
  config, and a bucket/prefix are selected. Local probe on 2026-05-18 found no
  Qdrant, Graphiti, Zep, S3, or MinIO endpoint environment configured and no
  matching Docker services already running.
- Provider-backed sandbox adapters are deferred until a provider can return
  validator-reviewable attestation evidence. They are not an active completion
  gate for this plan.
- Slack, tracker, PagerDuty, Google Calendar, Outlook, OpenTelemetry provider
  adapters, and other external-provider smokes are deferred behind approved
  credentials, webhook auth policy, and explicit activation gates.
- Persisted SPA screenshots, TUI transcripts, and optional LLM usability
  evaluators are deferred until the deterministic scenario runner has
  first-class browser/terminal capture paths and an approved evaluator route.
- Token-broker-backed multi-user MCP smoke is deferred because the default
  deployment remains single-user; it becomes active only after broker
  implementation, OIDC tenant/client config, short-lived lease policy, and
  approval UX are selected.
- Provider-specific deploy overlays and Restate Cloud journal-encryption
  guidance are deferred until a first cloud target and supported SDK/provider
  documentation path are selected.
- Additional live provider smokes for Claude Code, Bedrock, vLLM, Ollama,
  Hugging Face, and OpenAI-compatible gateways are deferred until those routes
  are configured on an approved node with credentials or brokered auth. The
  configured Codex App Server and Codex MCP routes were verified on 2026-05-18.

### Deferred TODOs

These items are intentionally not active `coat plan follow-ups` yet. Promote
one into `## Follow-Ups` only after its activation criteria are true and there
is a concrete proof path.

- [ ] Provider-backed sandbox adapters:
  - Select the first provider sandbox target and document the exact attestation
    shape it can return.
  - Add a `SandboxAttestation` fixture that a validator/reviewer can inspect
    without trusting provider prose.
  - Add an env-gated live smoke that launches a bounded executor, captures
    command/output/artifact refs, and fails if attestation evidence is missing
    or non-verifiable.
  - Promotion criteria: provider docs/API prove attestation support, credentials
    are approved, and the validator can reject missing or malformed evidence.
- [ ] Scenario artifact capture and usability evaluator:
  - Extend the deterministic scenario runner with first-class browser
    screenshot, Playwright trace, and TUI transcript artifact slots.
  - Store artifact refs in scenario evidence and operator snapshots without
    requiring the UI to expose raw paths in normal workflows.
  - Define a deterministic usability rubric before adding any LLM evaluator.
  - Add an optional LLM evaluator only behind explicit model/auth gates and keep
    PR CI deterministic by default.
  - Promotion criteria: scenario artifacts are captured by the runner, not ad
    hoc test scripts, and evaluator output has a stable schema.
- [ ] Token-broker-backed multi-user MCP smoke:
  - Choose the token broker design, OIDC tenant/client setup, lease duration,
    refresh policy, and revocation model.
  - Add `UserPrincipalRef`, broker lease refs, and MCP auth refs to smoke
    fixtures without storing raw tokens in state, logs, memory, or artifacts.
  - Add an approval UX for brokered user auth before a runner can use delegated
    credentials.
  - Prove single-user mode remains the default and multi-user OIDC is opt-in.
  - Promotion criteria: broker implementation exists, OIDC config is available,
    short-lived leases are testable, and the MCP smoke can authenticate as a
    delegated user without leaking secrets.

## Acceptance

- `coat plan follow-ups` lists this master plan as the live-runtime coordination point.
- Completed subsystem plans preserve their implementation evidence while this master plan owns residual live-runtime follow-ups.
- Replay tests run in normal CI without live credentials.
- Live tests are env-gated and skip clearly when credentials or external services are unavailable.
- A real Restate restart/resume test proves completed durable steps are not re-executed.
- A real Codex App Server task returns structured evidence, checkpoints, and replayable artifacts.
- A kind/k3d executor Job runs from coordinator-approved state through the sandbox-runner provision API, is watched to completion, returns structured result and attestation evidence, and is projected into goal-store.
- SQS LocalStack proves durable inbound and outbound event/notification behavior.
- Event gateway proof runs against Compose or cluster topology.
- Live provider verification records one profile result per enabled provider route.
- Staff-engineer remains gated until package behavior, isolated target repo install, tracker auth, and Claude Code auth distribution are verified.
- Live worker outputs are captured as reviewer/validator fixtures with git checkpoint/worktree evidence where branch workflows are involved.
- Browser E2E proves operator workflows against backend APIs rather than
  projection mutation, and failed PR-gated scenario runs expose
  `target/coat-scenarios` plus Playwright traces/screenshots for review.
- Published binary and Helm chart smoke evidence is recorded after the first release.
