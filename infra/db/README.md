# Operational Database

The durable source of execution truth remains Restate. The database schema here is the queryable operational mirror for dashboards, audit queries, joins across goals/events/artifacts, and optional local memory fallback.

Recommended production shape:

- PostgreSQL 16 or later for relational goal, task, event, approval, artifact, and outbox state.
- `pgvector` when you want an embedded local memory index in the same operational database. The scaffold uses `halfvec(3072)` so OpenAI `text-embedding-3-large`-sized embeddings can use a pgvector HNSW cosine index.
- Qdrant as the default dedicated vector database for high-volume semantic memory.
- Graphiti as the temporal knowledge graph layer when memories need time-aware entity relations.

Local Compose exposes PostgreSQL only under the `db` profile:

```sh
docker compose -f infra/compose/docker-compose.yml --profile db up postgres
```

The migration files are mounted into `/docker-entrypoint-initdb.d` for first boot. Production deployments should run the same files with a migration tool such as Atlas, Flyway, Liquibase, Sqitch, or refinery.

`coat-goal-store` uses the JSONL projection by default. To exercise the standard Postgres read model locally:

```sh
COAT_GOAL_STORE_BACKEND=postgres \
  docker compose -f infra/compose/docker-compose.yml --profile db up postgres goal-store
```

The service requires `COAT_GOAL_STORE_DATABASE_URL` or `DATABASE_URL` in Postgres mode. Compose provides a local default pointing at the profile database; production should inject it from a secret manager.

Restate handlers should write externally visible state through durable activities. Do not write directly from replayed workflow code unless the write is protected by idempotency keys or Restate side-effect recording.
