# 110: Protobuf Goal Store Protocols

## Goal

Add Buf-managed protobuf contracts and a durable goal-store projection path so operators, dashboards, runners, and future SDK clients can inspect goal progress through stable protocols without taking authority away from Restate.

## Scope

- Add `buf.yaml` and protocol files under `proto/coat/v1`.
- Keep JSON schemas as the Rust domain source of truth for in-process payloads.
- Add `GoalRecord`, `TaskRecord`, `GoalEventRecord`, approval, artifact, snapshot, and goal-store policy types.
- Add `DurablePlanRecord` and plan upsert/list/get/compile RPCs for durable planning mode.
- Add a local `coat-goal-store` service with JSONL replay for Compose smoke tests.
- Wire coordinator projections through durable Restate `ctx.run` steps.
- Add Compose, Kubernetes, CLI, and docs entrypoints.
- Add direct artifact-record append support for git results, object refs, and artifact manifests.

## Production Direction

Use Postgres as the standard operational read model. Add pgvector only when semantic search over operational records is needed. Keep Qdrant for dedicated embedded memory and RAG, Graphiti/Zep for temporal semantic memory, S3-compatible storage for large artifacts, and Restate for workflow authority.

Initial DDL lives in `infra/db/migrations/001_goal_store.sql` and `infra/db/migrations/003_memory_index.sql`. Compose exposes `pgvector/pgvector:pg16` through the optional `db` profile for local schema and dashboard development.

## Implemented Postgres Backend

`coat-goal-store` now supports `COAT_GOAL_STORE_BACKEND=postgres` with `COAT_GOAL_STORE_DATABASE_URL` or `DATABASE_URL`. In Postgres mode the service verifies that `coat.goals` exists, writes snapshot projections transactionally, stores exact record JSONB payloads for plans, goals, tasks, events, approvals, and artifacts, and reads the operator query endpoints back from Postgres.

`POST /goal-store/artifacts` and `coat store record-artifacts` append artifact refs without replacing the full projected snapshot.

Local JSONL replay mirrors the Postgres event idempotency rule: appending the same goal event sequence or idempotency key replaces the existing projected event instead of duplicating it. This keeps Restate replay-safe projection behavior consistent across local and Postgres backends.

## Local Schema And Protobuf Drift Discipline

`make proto-check` is the local and CI gate for protocol drift. It snapshots the current `schemas/` tree, regenerates JSON schemas from the Rust domain contracts, fails when regeneration changes that snapshot, runs `buf lint`, and verifies protobuf formatting with Buf's diff-only format check. That catches stale generation without requiring a clean local worktree when schema changes are intentionally part of the current patch.

CI now calls the same `make proto-check` target so protobuf linting, protobuf formatting, and JSON schema freshness stay together. Generated SDKs remain out of the repository until the final Rust and TypeScript SDK targets are selected.

## Follow-Ups

- Coordinate remaining Restate restart proof through `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`; SDK generation is scaffolded as an internal validation target and publishing is deferred by the active-plan SDK decision.
- Run the real Docker Testcontainers Restate restart/resume proof through the active `RuntimeVerifier` follow-up; keep it env-gated until Docker availability and the pinned Restate test image are in place.
