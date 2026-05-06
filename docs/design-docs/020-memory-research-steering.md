# Design Doc: Human-Steered Continuity, Research, And Memory

## Intent

COAT should feel like it can keep working, but it must stay steerable. The durable coordinator accepts human steering directives, can inject research tasks, and can pause, resume, or cancel goals without giving any worker an unbounded autonomous shell loop.

## Control Loop Policy

`GoalSpec.control_policy` defines the continuity envelope:

- `bounded_until_satisfied`: normal mode; stop when satisfied, blocked, cancelled, budget-exhausted, or paused.
- `monitor_until_cancelled`: return idle state instead of hard-blocking when no frontier is available.
- `human_steered_continuous`: allow repeated operator steering, injected tasks, and research requests while budgets still apply.

`SteeringDirective` is the operator control surface. It supports adding constraints, updating the objective, injecting bounded tasks, requesting research, pausing, resuming, and cancelling. The coordinator is still the only component that mutates the durable task tree.

## Research Direction

`GoalSpec.research_policy` requires research workers to be question-first:

- state the question being answered;
- search web/docs/MCP/memory/repo sources as allowed by policy;
- prefer primary, official, or peer-reviewed sources;
- return `ResearchOutput` with sources, confidence, open questions, and an `InformationUsePlan`;
- use gathered information by proposing task updates, validation checks, and facts to avoid.

Research output is not just prose. Validation requires source artifacts and an information-use plan for research tasks.

## Memory Substrate

The default memory substrate is Zep/Graphiti over MCP.

Reasons:

- Zep documents a temporal knowledge graph, fact invalidation, context blocks, graph search, and agentic tool use for retrieval.
- Graphiti is an open source temporal knowledge graph framework tailored for AI agents, and its MCP server exposes episode management, entity/relationship search, hybrid search, and graph maintenance over HTTP.
- Graphiti can run with FalkorDB by default and supports Neo4j when a production graph database is needed.

Restate remains the durable execution journal. Graphiti/Zep is the durable semantic memory layer. Postgres remains optional for queryable audit/index tables, not the primary agent memory.

The scaffold includes `coat-memory-gateway` as the local adapter boundary. It exposes REST endpoints and MCP-shaped tools:

- `memory_write`
- `memory_search`
- `memory_context`
- `memory_join`
- `memory_repair`
- `memory_events`

The gateway stores memory records in process memory and can replay an append-only JSONL journal when `MEMORY_GATEWAY_JOURNAL_PATH` is configured. When `MEMORY_GATEWAY_GRAPHITI_MCP_URL` is configured, it mirrors operations into Graphiti over MCP using the current `add_episode`, `search_nodes`, and `search_facts` tool names. When Qdrant and an embedding endpoint are configured, it also mirrors memory writes into vector memory. The mirrors are best-effort: responses include `adapter_reports`, `memory_context` returns bounded context packs with an `InformationUsePlan`, `memory_repair` can replay local records into restored adapters, and local memory remains the availability boundary. Postgres is used only for operational audit tables if needed.

## Fork/Join Memory Rules

Every forked task inherits parent memory context by reference, not by copied tokens. Branch workers may write branch-scoped memories when policy allows it. Review and unification tasks are responsible for deciding which branch facts become shared goal memory.

Default policy:

- inherit parent context;
- allow branch memories;
- write reviewed facts only;
- join with unifier-curated memory.

This prevents a failed actor branch from polluting shared durable memory before the critic/unifier loop has reviewed it.

## MCP And Auth

Memory access should be an MCP server in the task's `McpContextRef`, usually named `graphiti-memory`. Auth stays in `SecretRef` or workload identity. Runners receive the server ref and resolve credentials locally.

## Sources

- Zep concepts: `https://help.getzep.com/concepts`
- Zep Knowledge Graph MCP: `https://www.getzep.com/product/knowledge-graph-mcp/`
- Graphiti MCP server: `https://github.com/getzep/graphiti/tree/main/mcp_server`
- Zep paper: `https://arxiv.org/abs/2501.13956`
