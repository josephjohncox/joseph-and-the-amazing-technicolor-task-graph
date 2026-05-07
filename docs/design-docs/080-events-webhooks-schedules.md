# Design Doc: Events, Webhooks, Calendars, And Schedules

External events should not call worker agents directly. They enter through `coat-event-gateway`, become normalized events with dedupe keys, and then create or steer durable goals through Restate.

## Decision

Use standard event and schedule shapes:

- CloudEvents-compatible metadata for webhook and bus events.
- Generic JSON event sources with JSON Pointer extraction for event id, type, subject, and dedupe keys.
- AsyncAPI for documenting event-driven APIs; the current contract lives at `docs/api/event-gateway.asyncapi.yaml`.
- Kubernetes CronJobs for cluster-native scheduled trigger jobs.
- Restate timers and workflows for durable waits, retries, approval pauses, and follow-up scheduling inside a running goal.
- Provider-specific push APIs for calendars where available, plus bounded polling for calendar windows and missed notifications.

`coat-event-gateway` is the ingress service. It records event sources, raw events, and triggered-goal decisions in an append-only JSONL journal for local development, or in the Postgres event inbox/outbox when `COAT_EVENT_GATEWAY_BACKEND=postgres`. High-volume channels can still be bridged through Kafka, Redpanda, NATS, SQS/SNS, Pub/Sub, EventBridge, or another existing event bus. The Postgres schema lives in `infra/db/migrations/002_event_gateway.sql`.

## Topology

```mermaid
flowchart TD
    W["Webhook / CloudEvent"] --> EG["coat-event-gateway"]
    C["Calendar Push / Poll"] --> EG
    CR["Cron / Kubernetes CronJob"] --> EG
    Q["Queue / PubSub / Event Bus"] --> EG
    EG --> DEDUPE["Dedupe + Route Policy"]
    DEDUPE --> HR["Human Review Gate"]
    DEDUPE --> R["Restate GoalWorkflow"]
    R --> TT[("Durable Task Tree")]
    EG --> GS["Goal Store Projection"]
```

## Contracts

Core domain types:

- `EventSource`
- `WebhookEventSource`
- `GenericEventSource`
- `ScheduledEventSource`
- `CalendarEventSource`
- `ExternalEvent`
- `EventGoalRoute`
- `GoalTriggerTemplate`
- `TriggeredGoalRequest`
- `TriggeredGoalResponse`

Proto definitions live in `proto/coat/v1/eventing.proto`.

`EventGoalRoute.mode` decides whether an event is recorded only, creates a goal, creates a research goal, steers an existing goal, or waits for human review. Routes must carry dedupe windows and may require approval.

## Webhook Rules

Webhook handlers should:

- verify gateway auth before recording or triggering work;
- verify source auth through `WebhookAuthPolicy`;
- support shared-secret headers, bearer tokens, and HMAC-SHA256 signatures with secret values resolved from `SecretRef`;
- use provider HMAC presets for GitHub (`x-hub-signature-256`), Slack Events (`x-slack-signature` over `v0:{timestamp}:{body}`), and Stripe-style webhooks (`Stripe-Signature` over `{timestamp}.{body}`);
- terminate production-only mTLS or OIDC JWT flows at trusted ingress or secret middleware until a provider adapter is installed;
- normalize headers and payload into `ExternalEvent`;
- preserve provider delivery IDs as `dedupe_key`;
- never put raw shared secrets into event payloads, diagnostics, goal JSON, artifacts, or memory;
- create goals only through the event gateway and Restate ingress.

Provider-specific webhook adapters should map delivery metadata into CloudEvents-style fields: `id`, `source_id`, `event_type`, `subject`, `occurred_at`, and payload. The local gateway already has provider signature presets; full provider adapters should build on those presets instead of reimplementing auth.

## Generic Event Sources

Generic event sources let agents and outside systems respond to events that do not deserve a bespoke adapter yet: CI results, git branch updates, issue tracker changes, chat commands, monitoring alerts, database change notifications, memory updates, runner heartbeats, or agent topology proposals.

Use this pattern:

1. Register an `EventSource` with `kind=generic`, `ci`, `git`, `issue_tracker`, `chat`, `monitoring_alert`, `database_change`, `agent_event`, `runner_event`, `goal_lifecycle`, or `memory_event`.
2. Configure `generic.allowed_event_types` and JSON Pointer fields such as `/id`, `/type`, `/subject`, and `/delivery_id`.
3. Emit raw JSON or CloudEvents-compatible JSON to `POST /events/generic/{source_id}`.
4. The gateway normalizes the payload into `ExternalEvent`, applies dedupe, and routes according to `EventGoalRoute`.

`POST /events` still accepts a fully normalized `ExternalEvent`; add `?route=true` when an operator or upstream bus wants the gateway to route the event from its registered source policy. `POST /events/generic/{source_id}` routes by default unless `?route=false` is set.

Use generic sources as the first integration boundary. Add provider-specific adapters only when auth, normalization, rate limiting, or schema validation is stable enough to justify a typed adapter.

## Calendar And Scheduled Work

Calendar checks are event sources, not hidden loops inside workers.

Use this pattern:

1. A Kubernetes CronJob, Restate timer, or small scheduler calls the event gateway.
2. The gateway records a calendar-window event.
3. A route either creates a planning/research goal or requests human review.
4. The goal runs through normal task distribution, approval, memory, research, and validation.

The example cluster schedule is `infra/k8s/examples/calendar-trigger-cronjob.yaml`; it is suspended by default so operators must explicitly activate it after source, route, and auth review.

For Google Calendar or Outlook:

- use provider push notifications where reliable and configured;
- keep sync tokens and OAuth/session material in `SecretRef` or workload identity;
- do bounded polling with `lookahead_seconds` and `poll_interval_seconds` when push notifications are not enough;
- emit a durable event for each actionable calendar window, not for every raw provider notification.

## Cron And Event Loops

Use three layers:

- Cluster schedules: Kubernetes CronJobs for detached scheduled triggers.
- Durable waits: Restate timers and workflow state for "wake this goal later" behavior.
- Operator automations: local Codex thread heartbeats/reminders only when the request is specifically about this interactive thread.

Do not create a long-lived agent process that sleeps and wakes itself forever. Every wakeup must become an `ExternalEvent`, `TriggeredGoalRequest`, steering directive, or bounded task.

## Agent-Generated Topologies

Agents may propose new event sources, routes, goals, or schedules, but they do not activate them directly. They return structured child requests or artifacts. The coordinator or a human-approved operations task installs the event source.

This allows dynamic topologies while preserving auditability:

- an incident reviewer can propose a follow-up monitor;
- a research agent can propose weekly source refreshes;
- a release unifier can propose post-release health checks;
- a calendar assistant can propose a recurring planning goal.

Activation requires policy validation and, for webhooks, external callbacks, calendar auth, or schedule creation, human approval by default.

Set `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` in production-like environments. When enabled, `coat-event-gateway` rejects activation of risky enabled sources unless the registration request includes `x-coat-approval-id`; `coat event register --approval-id ...` sets that header. Operators can still register the source disabled first and activate it after review.
