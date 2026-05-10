# Design Doc: Distributed Memory And Durable Knowledgebases

COAT has two different kinds of durability:

- Restate durability: workflow replay, side-effect journaling, task state, and pause/resume.
- Knowledge durability: reusable facts, decisions, relationships, source evidence, and branch-specific memories shared across agents and runs.

Do not collapse these into one store. Restate owns execution truth. The memory layer owns semantic context.

## Recommended Stack

Default:

- Zep/Graphiti over MCP for temporal agent memory.
- Qdrant for production vector memory and RAG retrieval.
- FalkorDB for the simple local Graphiti-backed graph path.
- Neo4j when teams need an enterprise graph database, graph tooling, or Neo4j GraphRAG integration.
- Postgres plus pgvector for audit, relational metadata, vector search over operational records, and queryable indexes.

Specialized additions:

- LanceDB for embedded or lakehouse-style multimodal retrieval with official Python, TypeScript, and Rust SDKs.
- Tantivy for Rust-native local full-text indexing where running a search service is not justified.

These are intentionally common, well-supported pieces. Avoid designing the first production path around a niche memory framework unless it has clear maintainer health, durable storage, auth, observability, backup/restore, and client support.

## Source-Backed Rationale

- Zep documents temporal knowledge graphs, agent memory, context engineering, SDKs, and MCP-facing docs.
- Graphiti is an open source temporal knowledge graph framework for AI agents and has an MCP server for agent memory.
- FalkorDB documents Graphiti MCP usage and positions itself as a graph database with OpenCypher, vector similarity, and full-text search.
- Neo4j ships a first-party GraphRAG package and documents long-term maintenance for it.
- pgvector is the standard Postgres extension for vector similarity search while keeping relational database properties.
- Qdrant documents official clients including Rust, REST/gRPC interfaces, collection creation, point upsert, and query/search APIs.
- OpenAI documents `text-embedding-3-large` and `text-embedding-3-small`, including default dimensions and the embeddings API.
- Hugging Face Text Embeddings Inference documents OpenAI-compatible embedding serving for local open source embedding models.
- LanceDB documents official Python, TypeScript, and Rust SDKs.
- Tantivy is a widely used Rust full-text search engine library inspired by Lucene.

See `docs/references/source-links.md` for the current source list.

## Memory Planes

### Execution Journal

Restate records durable workflow progress, model calls, service calls, timers, and child task state. This is not a semantic memory store. Do not ask workers to query Restate for "what do we know about this repo?" except through coordinator-owned status APIs.

### Memory Gateway

`coat-memory-gateway` is the stable local API. It exposes:

- `memory_write`
- `memory_search`
- `memory_context`
- `memory_join`
- `memory_repair`
- `memory_events`

The gateway provides REST and MCP-shaped endpoints so Codex, Agents SDK workers, Rust services, and local model runners can all use the same contract. Workers should prefer `memory_context` over raw `memory_search` before substantial work because it returns ranked hits plus deterministic guidance about what to use, what to avoid, and which validation checks to preserve.

### Semantic Memory

Zep/Graphiti stores episodes, facts, temporal relationships, and invalidations. Workers write scoped memories with provenance. Reviewers and unifiers promote or invalidate branch memories after evaluation.

### Vector Memory

Qdrant stores embedded memory episodes and retrieval chunks. The gateway mirrors reviewed `memory_write` and `memory_join` records into Qdrant when `MEMORY_GATEWAY_QDRANT_URL`, `MEMORY_GATEWAY_EMBEDDING_URL`, and `MEMORY_GATEWAY_EMBEDDING_MODEL` are configured together. Qdrant failures are returned in `adapter_reports` and do not roll back the local journal or Graphiti write.

Use Qdrant for cross-run RAG, semantic search over memories, source chunks, repository summaries, and branch context. Use filters for `goal_id`, scope, repo, persona, and provenance so distributed agents do not retrieve another goal's private context by accident.

### Queryable Audit

Postgres should hold queryable operational records: goals, task summaries, approval decisions, runner registrations, dispatch choices, validation scores, memory event indexes, and artifact metadata. Add pgvector when semantic search over these records is useful. For 3072-dimensional embedding models, use pgvector `halfvec` indexes rather than plain `vector` HNSW indexes.

### Retrieval Stores

Use a vector or full-text store for source documents, codebase snapshots, generated docs, and long-lived corpora. Keep retrieval stores separate from approval and task truth.

## Embedding Model Policy

Use standard embedding providers. Do not invent embedding algorithms or custom vector formats.

Recommended hosted path:

- OpenAI `text-embedding-3-large` for high-quality memory retrieval.
- OpenAI `text-embedding-3-small` when lower cost and smaller vectors matter more than maximum recall.

Recommended local/open source path:

- Hugging Face Text Embeddings Inference for BGE, E5, GTE, Qwen embedding, or other supported open source embedding models.
- Prefer TEI's OpenAI-compatible `/v1/embeddings` endpoint so the gateway, sidecars, and future workers can share one adapter shape.
- Ollama, vLLM, llama.cpp, Hugging Face endpoints, and other OpenAI-compatible providers should be configured through `coat setup local-auth` so model IDs come from live endpoint discovery instead of compiled-in examples.

Operational rules:

- Store the embedding model and dimensions in `MemoryPolicy.embedding`.
- Re-index collections when changing embedding model, dimensions, or normalization policy.
- Keep embedding tokens in `SecretRef`, environment variables, Kubernetes Secrets, Vault, cloud secret managers, or workload identity. Do not put tokens in goal JSON.
- If using local vLLM or other OpenAI-compatible chat providers, keep embedding provider selection independent; chat model routing and memory embeddings do not have to use the same model.

## Default RAG Retrieval Flow

The retrieval flow should be explicit and reviewable:

1. Search local gateway records lexically for fresh, journal-backed facts.
2. Search Graphiti/Zep for temporal facts, relationships, invalidations, and prior decisions.
3. Search Qdrant for embedded memories and source chunks.
4. Optionally search Postgres/pgvector for operational records when SQL joins matter.
5. Optionally search Tantivy or a service-backed full-text index for exact terms, identifiers, and code/doc snippets.
6. Fuse results with `MemoryRetrievalPolicy.fusion`, then return or write an `InformationUsePlan`.

Use graph-then-vector retrieval for evolving facts and vector-then-graph retrieval for source-heavy RAG. Use hybrid RRF or reranking only when there is enough retrieval volume to measure quality.

## Context Distribution

Workers receive context by reference:

- `goal_id`
- `task_id`
- `McpContextRef`
- `MemoryStoreRef`
- `SecretRef`
- allowed memory scopes
- artifact references

Workers should not receive raw secrets, copied database credentials, or giant prompt dumps of prior state.

The runner resolves MCP auth locally using environment variables, Kubernetes Secrets, Vault, cloud secret managers, workload identity, or OAuth delegation. The coordinator may issue short-lived context tokens when `McpContextPropagation::CoordinatorIssued` is selected.

## Fork/Join Memory Rules

Every forked task inherits memory context by reference.

Actor, researcher, tester, and reviewer branches may write branch-scoped memories when policy allows it. These writes must include:

- goal ID;
- task ID;
- scope;
- source actor;
- evidence URI;
- confidence when applicable;
- tags;
- whether the memory is branch-only or promoted.

Unifier tasks are responsible for joins:

- promote accepted facts;
- invalidate contradicted or stale branch facts;
- write a join summary;
- preserve source evidence;
- leave rejected branch memory searchable for audit but excluded from default shared context.

This prevents failed branches from poisoning future work.

## Memory Retrieval At Task Start

A runner should perform this sequence before doing substantial work:

1. Read the task objective, purpose, done criteria, and execution profile.
2. Query memory for goal, repo, persona, and parent-task scopes.
3. Search source knowledgebases if research policy allows it.
4. Build a small `InformationUsePlan` with usable facts, rejected assumptions, proposed memory writes, proposed child tasks, and validation checks.
5. Use only the facts that have provenance and fit the task scope.
6. Record missing context as open questions or child research requests.

For coding tasks, memory should bias the worker toward known repo rules, prior failed attempts, accepted architecture decisions, and known test commands. It should not override current source code.

## Idempotency And Durability

Memory writes need stable keys. Use goal/task IDs plus a content-purpose suffix when possible. External graph writes should be best-effort and idempotent. The local gateway journal remains the availability boundary for development.

Recommended write pattern:

```text
local journal append
local in-memory/index update
best-effort external graph MCP write
best-effort vector embedding + Qdrant upsert
return adapter_reports
```

Do not roll back local memory because an external graph service is temporarily unavailable. Instead, expose the failed adapter report and let an operator or replay job repair the mirror later.

The gateway exposes `memory_repair` for this repair path. It scans local journal-backed records, filters by goal or key, and replays selected records into configured adapter stores such as Graphiti and Qdrant. Run it in dry-run mode first, then repair one store kind at a time when credentials, embedding endpoints, or external services have been restored.

## Service Choices

Use Zep/Graphiti when:

- facts and relationships change over time;
- agents need temporal context across sessions;
- memory is about conversations, decisions, goals, requirements, and evolving relationships;
- MCP access is the clean integration path.

Use Neo4j when:

- graph operations, graph visualization, Cypher skills, enterprise support, or existing Neo4j operations matter;
- GraphRAG integration is a core production requirement.

Use FalkorDB when:

- you want a lightweight Graphiti-friendly graph database path;
- OpenCypher, vector similarity, full-text search, and local container deployment are enough.

Use Postgres and pgvector when:

- relational audit, joins, transactions, backups, and simple vector search should live in one operational database;
- the organization already operates Postgres well.

Use Qdrant when:

- vector retrieval is large enough to justify a dedicated vector service;
- Rust clients and service-level deployment matter.

Use LanceDB when:

- embedded or lakehouse-style multimodal retrieval is useful;
- local/edge or dataset-oriented workflows need versioned tables and Rust/TypeScript/Python SDKs.

Use Tantivy when:

- Rust-native full-text search is enough;
- you need a local index for repo docs, generated artifacts, or memory event summaries.

## Anti-Patterns

- Treating vector search as the only memory layer for evolving facts.
- Copying all prior context into every worker prompt.
- Letting workers promote shared memory without reviewer or unifier approval.
- Storing auth tokens in goal JSON or memory episodes.
- Using Restate workflow state as a general knowledgebase.
- Making the coordinator parse unstructured memory prose instead of structured memory events.
- Losing rejected branch memories instead of retaining them for audit.

## Implementation Direction

Short term:

- Keep `coat-memory-gateway` as the only local memory contract.
- Continue JSONL replay for local development.
- Mirror to Graphiti only when `MEMORY_GATEWAY_GRAPHITI_MCP_URL` is configured.
- Repair Graphiti/Qdrant mirrors with `memory_repair` when an adapter was unavailable during the original write.
- Store memory keys and adapter reports in task artifacts and validation evidence.

Production:

- Run Graphiti/Zep memory with a managed graph backend.
- Run Qdrant as the default vector memory service for embedded RAG.
- Add Postgres for operational query/audit tables.
- Add pgvector when the Postgres audit database also needs SQL-native vector search.
- Add LanceDB when embedded or lakehouse-style multimodal retrieval is a better fit than a service.
- Add a memory repair worker that replays failed adapter reports.
- Add retention and invalidation policies by scope.
