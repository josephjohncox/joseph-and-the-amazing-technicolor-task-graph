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

## Follow-Ups

- Add generated Rust/TypeScript SDKs from Buf once the final SDK target is selected.
- Add integration tests proving Restate replay does not duplicate projected events.
