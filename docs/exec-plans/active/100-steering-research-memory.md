# 100 Steering Research Memory

## Objective

Add human-steered continuity, sourced research tasks, and durable fork/join memory policy without creating an unbounded autonomous loop.

## Implementation

- Add `ControlLoopPolicy`, `SteeringDirective`, and `ControlLoopMode`.
- Add `ResearchPolicy`, `ResearchOutput`, `SourceArtifact`, and `InformationUsePlan`.
- Add `MemoryPolicy`, `MemoryStoreRef`, `MemoryForkJoinPolicy`, `MemoryEvent`, `VectorMemoryPolicy`, `EmbeddingPolicy`, and `MemoryRetrievalPolicy`.
- Default semantic memory to Zep/Graphiti over MCP and Qdrant-backed embedded retrieval.
- Add a Restate `GoalWorkflow/steer` shared handler and `coat goal steer`.
- Validate research tasks for sources and an information-use plan.
- Add `coat-memory-gateway` with REST and MCP-shaped write/search/join/events tools.
- Add `memory_context` so workers can fetch bounded context packs with deterministic `InformationUsePlan` guidance.
- Add a best-effort Qdrant vector adapter using an OpenAI-compatible embedding endpoint.
- Add `memory_repair` so operators can replay local journal records into Graphiti or Qdrant after adapter outages.
- Add `coat memory` CLI commands and deployment wiring.

## Tests

- Steering can inject a research task.
- Research validation rejects missing sources or missing use plan.
- Memory gateway can write and search a local memory record.
- Memory gateway can return a task context pack from local memory hits.
- Memory gateway can build Qdrant filters and parse Qdrant query/search hits.
- Memory gateway dry-run repair counts selected records and adapter operations.
- Examples parse against the domain contracts.
- Schemas include control, steering, research, and memory contracts.

## Acceptance

- `cargo test --workspace` passes.
- `cargo run -p coat-domain --bin generate-schemas -- schemas` writes the new schemas.
- Operators can submit `examples/steering-request-research.json`.
