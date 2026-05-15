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
- Add `memory_retract` so operators and unifiers can invalidate stale or superseded facts without abusing the fork/join endpoint.
- Add `memory_edit` so operators can retract old keys and write a linked replacement fact in one reviewed operation.
- Add `memory_edit_preview` so operators can inspect replacement diffs before mutating durable memory.
- Add `coat memory` CLI commands and deployment wiring.
- Expose memory search, context, write, join, retract, edit, edit-preview, repair, event reads, and research-output application through the control gateway SPA and MCP surface.

## Tests

- Steering can inject a research task.
- Research validation rejects missing sources or missing use plan.
- Memory gateway can write and search a local memory record.
- Memory gateway can return a task context pack from local memory hits.
- Memory gateway can build Qdrant filters and parse Qdrant query/search hits.
- Memory gateway can retract selected local memory records and replay the retraction from the JSONL journal.
- Memory gateway can edit memory by retracting old keys and writing a replacement record.
- Memory gateway can preview memory replacement diffs before committing an edit.
- Memory gateway dry-run repair counts selected records and adapter operations.
- Live Qdrant and Graphiti adapter round-trip tests are gated by explicit env flags and service credentials, so normal CI remains local-only while real adapter validation is available on configured nodes.
- Control gateway exposes memory join, retract, edit, edit-preview, repair, event reads, and research-output steering helpers.
- Control gateway UI can preview memory replacements with before/after excerpts before applying a durable edit.
- Control gateway smoke renders the memory replacement preview status and before/after diff table against realistic ready and blocked edit-preview payloads.
- Examples parse against the domain contracts.
- Schemas include control, steering, research, and memory contracts.

## Follow-Ups

- Superseded by the active master plan:
  `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
  ResearchMemory owns live Graphiti/Zep and Qdrant adapter tests once service
  URLs and credentials are approved; UIE2E owns browser memory workflow proof.

## Acceptance

- `cargo test --workspace` passes.
- `make schemas` writes the new schemas.
- Operators can submit `examples/steering-request-research.json`.
