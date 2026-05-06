# 120: Events, Webhooks, And Scheduled Goals

## Goal

Add an event-ingress surface so external webhooks, calendar changes, scheduled triggers, event buses, and agent-proposed monitors can create or steer durable goals without bypassing the coordinator.

## Scope

- Add domain contracts for event sources, webhooks, schedules, calendars, routes, external events, and triggered goals.
- Add protobuf contracts in `proto/coat/v1/eventing.proto`.
- Add `coat-event-gateway` with local JSONL replay.
- Add CLI commands for event source registration, event ingestion, trigger submission, and inspection.
- Add Compose and Kubernetes deployment entries.
- Document how webhooks, calendars, cron, and event buses interact with Restate.

## Implemented

- Local event gateway records event sources, events, trigger responses, and dedupe keys in a replayed JSONL journal.
- Mutating endpoints can require gateway bearer auth through `COAT_EVENT_GATEWAY_TOKEN`.
- Registered webhook sources can require shared-secret headers, bearer tokens, or HMAC-SHA256 signatures with secrets resolved through `SecretRef`.
- Routes support record-only, create-goal, create-research-goal, steer-goal, and human-review modes.
- `COAT_REQUIRE_EVENT_SOURCE_APPROVAL=true` blocks activation of risky enabled sources unless `x-coat-approval-id` is supplied.
- Postgres inbox/outbox migrations are scaffolded under `infra/db/migrations/`.
- AsyncAPI docs live at `docs/api/event-gateway.asyncapi.yaml`.
- Kubernetes examples cover suspended CronJob triggers and optional pgvector-backed Postgres.

## Future Work

- Add provider-specific signature adapters for GitHub, GitLab, Slack, Stripe-style HMAC, and custom canonicalization rules on top of the generic HMAC verifier.
- Implement the Postgres event backend behind `coat-event-gateway`.
- Add Google Calendar and Outlook source adapters using MCP or provider APIs.
- Persist event-source approval records in the goal store instead of only requiring an approval reference header.
