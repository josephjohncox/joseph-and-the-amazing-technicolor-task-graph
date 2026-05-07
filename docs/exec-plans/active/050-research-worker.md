# 050 Research Worker

## Objective

Create a bounded research worker that can collect current facts without owning the task tree.

## Implementation

- Use MCP/search tools through a narrow adapter.
- Return source URLs, short evidence summaries, confidence, and child-task requests.
- Use `InformationUsePlan` to carry proposed goal updates, task requests, memory writes, validation checks, and optional review-doctrine recommendations.
- Force all child requests through coordinator spawn policy.
- Separate research artifacts from validation decisions.

## Tests

- Stub worker returns source-shaped artifacts.
- Live web/tool tests are gated and cite sources.
- Validator rejects uncited current-fact claims when citations are required.

## Follow-Ups

- Add live research adapters behind explicit network and citation gates, with source capture that can be replayed by reviewers.
- Add replay fixtures for live research source capture once the live adapter exists.

## Acceptance

- Research can run as a sidecar or Rust service.
- Research output conforms to `AgentRunResult`.
- `examples/research-output-memory-substrate.json` demonstrates research-to-plan updates for goal policy, child tasks, memory writes, and review doctrine.
