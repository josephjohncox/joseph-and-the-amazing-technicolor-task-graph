# 050 Research Worker

## Objective

Create a bounded research worker that can collect current facts without owning the task tree.

## Implementation

- Use MCP/search tools through a narrow adapter.
- Return source URLs, short evidence summaries, confidence, and child-task requests.
- Force all child requests through coordinator spawn policy.
- Separate research artifacts from validation decisions.

## Tests

- Stub worker returns source-shaped artifacts.
- Live web/tool tests are gated and cite sources.
- Validator rejects uncited current-fact claims when citations are required.

## Acceptance

- Research can run as a sidecar or Rust service.
- Research output conforms to `AgentRunResult`.
