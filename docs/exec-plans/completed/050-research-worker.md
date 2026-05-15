# 050 Research Worker

## Objective

Create a bounded research worker that can collect current facts without owning the task tree.

## Implementation

- Use MCP/search tools through a narrow adapter.
- Return source URLs, short evidence summaries, confidence, and child-task requests.
- Capture routed web/reference search responses as replayable `WebSearchResponse` fixtures with the original request, structured worker result, mirrored `ResearchOutput`, source artifacts, diagnostics, and information-use plan.
- Include object-artifact refs for raw source snapshots and fetch metadata in replay fixtures, so reviewer workflows can inspect planned raw captures without live object-store access.
- Use `InformationUsePlan` to carry proposed goal updates, task requests, memory writes, validation checks, and optional review-doctrine recommendations.
- Force all child requests through coordinator spawn policy.
- Separate research artifacts from validation decisions.

## Tests

- Stub worker returns source-shaped artifacts.
- Live web/tool tests are gated and cite sources.
- Validator rejects uncited current-fact claims when citations are required.
- `examples/web-search-response-replay.json` parses as `WebSearchResponse`, carries raw-capture object refs, and validates a captured sourced research result without live web access.

## Follow-Ups

- Superseded by the active master plan:
  `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
  ResearchMemory owns live research adapters, citation/source capture, replay
  fixtures, and promotion from raw-capture refs to object-store snapshots.

## Acceptance

- Research can run as a sidecar or Rust service.
- Research output conforms to `AgentRunResult`.
- `examples/research-output-memory-substrate.json` demonstrates research-to-plan updates for goal policy, child tasks, memory writes, and review doctrine.
- `examples/web-search-response-replay.json` demonstrates offline replay of a routed sourced research capture through validator-compatible result evidence.
