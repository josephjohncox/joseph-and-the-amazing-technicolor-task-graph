# Design Doc: Protobuf Protocols And Goal Store

Restate remains the source of truth for durable execution. The goal store is a queryable projection of that truth, not a competing scheduler or coordinator.

## Decision

Use Buf-managed protobuf contracts under `proto/coat/v1` for service boundaries and database-facing projection records.

Use Postgres as the standard production goal read model:

- goals, tasks, approvals, events, runner decisions, validation scores, and artifact refs live in relational tables;
- JSONB stores the full `GoalState`, `TaskNode`, or worker result payload when an operator needs exact contract replay;
- pgvector can be added for semantic search over operational records, but Qdrant remains the default dedicated vector memory service;
- S3-compatible object storage holds large artifacts;
- Restate owns workflow state, retries, timers, approvals, and durable child-task progress.

The local scaffold uses `coat-goal-store` with an append-only JSONL journal. That is a development read model and smoke-test target. Production should swap the backend for Postgres without changing the proto or JSON-schema envelopes.

## Protocol Layers

`proto/coat/v1/common.proto` defines shared IDs, statuses, worker roles, artifact refs, git refs, object-store refs, checkpoint refs, model candidates, budgets, done criteria, and protocol metadata.

`proto/coat/v1/goal_store.proto` defines:

- `GoalStoreService`
- `GoalRecord`
- `TaskRecord`
- `GoalEventRecord`
- `ApprovalRecord`
- `EventSourceApprovalRecord`
- `GoalStoreSnapshot`
- `DurablePlanRecord`
- snapshot upsert, durable plan upsert/list/get/compile, event append, goal/task/event/checkpoint query, event-source approval record/list, and artifact record RPCs.

HTTP local development mirrors the artifact and event-source approval RPCs with `POST /goal-store/artifacts` and `POST /goal-store/event-source-approvals`, allowing workers or smoke tests to append artifact, git-result, object-artifact, checkpoint, and ingress approval refs without rewriting a full snapshot.

`proto/coat/v1/runner.proto` defines:

- `AgentRunnerService`
- `RunnerRegistryService`
- runner registration, heartbeat, dispatch, task run, capabilities, model route, and result envelopes.

The protobuf messages intentionally use `JsonSchemaEnvelope` for the full Rust domain payload. That avoids duplicating the complete domain model twice while keeping query-critical fields strongly typed.

## Durable Write Pattern

The coordinator writes durable state in this order:

1. update Restate workflow state with `ctx.set`;
2. project a typed snapshot to the goal store inside a named durable `ctx.run` step;
3. continue the frontier loop only after the projection step is recorded.

Default projection mode is best effort. If `COAT_GOAL_STORE_REQUIRED=true`, projection failure becomes a terminal workflow error so production environments can require read-model durability.

## Idempotency

Every projection carries an idempotency key:

```text
goal:<goal_id>:projection:<reason>:<event_sequence>
```

Database writes should use upserts keyed by goal ID, task ID, event sequence, and idempotency key. Event append should be monotonic per goal. Replaying a completed Restate step must not insert a duplicate event or artifact row.

## Database Shape

Recommended Postgres tables:

- `plans(plan_id primary key, status, mode, title, objective, repo, version, compiled_goal_id, payload jsonb)`
- `goals(goal_id primary key, status, title, objective, repo, percent_done, satisfied, updated_at, payload jsonb)`
- `tasks(task_id primary key, goal_id, parent_task_id, subgoal_id, role, status, purpose_kind, depth, priority_rank, runnable, result_uri, payload jsonb)`
- `goal_events(goal_id, sequence, event_id, kind, task_id, message, actor, idempotency_key unique, payload jsonb)`
- `approvals(approval_id primary key, goal_id, task_id, status, risk, requested_action, payload jsonb)`
- `event_source_approvals(approval_ref, source_id, source_kind, status, risky, operator, payload jsonb)`
- `artifacts(goal_id, task_id, kind, uri, sha256, git_ref jsonb, object_ref jsonb, checkpoint_id, checkpoint_kind, checkpoint_label, payload jsonb)`

Indexes should cover `goal_id`, `status`, `role`, `subgoal_id`, `runnable`, `kind`, and `updated_at`. Add pgvector only for operational semantic search; do not put long-lived agent memory exclusively in the goal store.

## Query Rules

Workers do not query the goal store for authority. They ask the coordinator for assigned tasks and use memory/MCP context refs for task context.

Operators and dashboards query the goal store for:

- durable planning drafts and compiled plan outputs;
- progress summaries;
- subgoal/task distribution;
- approval queues;
- blocked work;
- event timelines;
- artifact refs;
- checkpoint history;
- cross-goal audit and reporting.

## Deployment

Compose runs `goal-store` on `:9088` with `COAT_GOAL_STORE_BACKEND=jsonl` and `COAT_GOAL_STORE_JOURNAL_PATH=/data/goal-store.jsonl` by default. The same binary supports `COAT_GOAL_STORE_BACKEND=postgres` with `COAT_GOAL_STORE_DATABASE_URL`, writing typed projection columns plus exact `record_json` JSONB payloads.

Kubernetes includes a separate `goal-store` Deployment and Service. Production clusters should set the backend to Postgres and source credentials from Kubernetes Secrets, External Secrets, Vault, cloud secret managers, or workload identity.

## Verification

Use:

```sh
buf lint
make schemas
cargo test --workspace
coat store policy
```
