# 120: Events, Webhooks, And Scheduled Goals

## Goal

Add an event-ingress surface so external webhooks, calendar changes, scheduled triggers, event buses, and agent-proposed monitors can create or steer durable goals without bypassing the coordinator.

## Scope

- Add domain contracts for event sources, generic JSON sources, observability sources, webhooks, schedules, calendars, routes, external events, and triggered goals.
- Add protobuf contracts in `proto/coat/v1/eventing.proto`.
- Add `coat-event-gateway` with local JSONL replay.
- Add CLI commands for event source registration, event ingestion, trigger submission, and inspection.
- Add Compose and Kubernetes deployment entries.
- Document how generic events, observability events, webhooks, calendars, cron, and event buses interact with Restate.

## Implemented

- Local event gateway records event sources, events, trigger responses, and dedupe keys in a replayed JSONL journal.
- Mutating endpoints can require gateway bearer auth through `COAT_EVENT_GATEWAY_TOKEN`.
- Registered webhook sources can require shared-secret headers, bearer tokens, or HMAC-SHA256 signatures with secrets resolved through `SecretRef`.
- Provider HMAC presets verify GitHub, Slack Events, and Stripe-style canonical signatures without exposing raw secrets in event state.
- Webhook ingestion normalizes GitHub, GitLab, Slack Events, Stripe-style, Jira, Linear, Prometheus Alertmanager, and Datadog monitor payloads into stable `ExternalEvent` IDs, event types, subjects, dedupe keys, and observability metadata.
- Observability templates expose service, severity, alert name, environment, runbook, dashboard, and JSON Pointer placeholders so live system events can route to SRE, software-engineering, data-engineering, and data-science goals that use durable memory before proposing PRs or dashboard/trigger changes.
- Registered generic event sources can normalize arbitrary JSON or CloudEvents-compatible payloads through JSON Pointer extraction and route through the same policy.
- `coat event emit --source-id ... --file ...` emits generic events and routes them by default.
- Routes support record-only, create-goal, create-research-goal, steer-goal, and human-review modes.
- `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` blocks activation of risky enabled sources unless `x-coat-approval-id` is supplied.
- Accepted event-source activation approval references are projected into the goal store as ingress-scoped `EventSourceApprovalRecord`s when `COAT_GOAL_STORE_URL` is configured.
- Postgres inbox/outbox migrations are scaffolded under `infra/db/migrations/`.
- `COAT_EVENT_GATEWAY_BACKEND=postgres` writes and reads event sources, external events, and triggered-goal responses through the Postgres event tables.
- AsyncAPI docs live at `docs/api/event-gateway.asyncapi.yaml`.
- Kubernetes examples cover suspended CronJob triggers and optional pgvector-backed Postgres.

## Follow-Ups

- Add Google Calendar and Outlook source adapters using MCP or provider APIs.
- Add OpenTelemetry log/metric/trace and additional provider adapters after the generic `open_telemetry_signal` shape stabilizes.
- Add an end-to-end gateway-to-goal-store projection smoke test once the Compose harness can run in CI.
