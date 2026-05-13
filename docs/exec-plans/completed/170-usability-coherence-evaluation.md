# 170 Usability Coherence Evaluation

## Objective

Define a product-level usability evaluation for COAT operator workflows. The
evaluation verifies that an operator can understand the goal, current task
graph, blocked states, next action, evidence, and completion status from the
SPA, TUI, CLI scenario report, and captured artifacts without knowing JSON,
protobuf, Restate workflows, or internal service topology.

This plan owns the rubric, deterministic scenario checks, SPA/TUI operator
journey coverage, and CI wiring for the first usability/coherence pass.

## Status

Completed for the deterministic PR-gated pass on 2026-05-12. The implementation
adds scenario-level coherence expectations, browser-level operator journeys,
accessibility checks, SPA evidence and next-action panels, and TUI selected-goal
runtime context.

Residual screenshot/transcript capture and optional LLM usability evaluation
work is preserved in `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`
under the `UIE2E` follow-ups.

## Evaluation Scope

- Primary operator path: create or select a goal, inspect progress, identify
  blocked work, choose the next action, inspect evidence, and determine whether
  the goal is complete or still needs work.
- Surfaces under review: `coat scenario report`, scenario evidence artifacts,
  the TypeScript SPA, and `coat tui`.
- Out of scope: live model quality, provider credentials, Restate internals,
  data-model completeness beyond what the operator needs to act, and aesthetic
  redesign unrelated to operator comprehension.
- Baseline user: an engineering operator who understands goals, tasks,
  approvals, evidence, and completion criteria, but does not know COAT internals.

## Rubric

Each scenario review scores every dimension as `0` fail, `1` partial, or `2`
pass. A PR-gated scenario fails the usability review if any hard-gate dimension
scores `0`, even when endpoint or screenshot smoke checks pass.

| Dimension | Hard Gate | Passing Evidence |
| --- | --- | --- |
| Goal orientation | Yes | The operator can name the active goal, objective, done criteria, owner surface, and selected goal context from the first visible SPA or TUI view. |
| Current task graph | Yes | The operator can see active, completed, failed, waiting, review, and child-task relationships with human-readable labels and without opening raw payloads. |
| Blocked states | Yes | Blocked or waiting work explains the reason, who or what can unblock it, and whether the wait is human input, approval, external event, capacity, model availability, or task failure. |
| Next action | Yes | The UI and report expose one clear next operator action when action is needed, or state that no operator action is currently required. |
| Evidence trail | Yes | Evidence is reachable from the task or goal view, grouped by command, review, artifact, checkpoint, browser trace, or source reference, and includes enough context to judge relevance. |
| Completion status | Yes | The operator can distinguish complete, incomplete, blocked, failed, cancelled, and budget-exhausted outcomes, including the reason a goal is or is not satisfied. |
| No internal knowledge required | Yes | The primary path uses product terms and does not require interpreting raw JSON, protobuf field names, Restate workflow IDs, database rows, or service-specific internals. |
| SPA and TUI coherence | Yes | SPA and TUI show the same selected goal, major task counts, blocked work, next action, and completion state for the same scenario run. |
| Failure diagnosis | No | When a scenario fails, artifacts distinguish product workflow failure from fixture, service, browser, or harness failure. |
| Reviewability | No | The scenario report and artifact tree are deterministic enough for a reviewer to reproduce the same evaluation locally. |

## Acceptance Criteria

- At least one checked-in scenario is evaluated against this rubric before the
  usability/coherence pass is marked complete.
- The scenario report contains a human-readable usability summary with answers
  to these operator questions:
  - What goal am I looking at?
  - What work exists now, and what is already done?
  - What is blocked, and why?
  - What should I do next?
  - What evidence supports the current state?
  - Is the goal complete, and if not, what remains?
- The SPA and TUI evidence for the same run agree on selected goal identity,
  major status counts, blocked or waiting work, and next action.
- Raw JSON may exist as an artifact for developers, but the primary review path
  must be understandable from labels, summaries, screenshots, and reports.
- Operator-facing text avoids requiring knowledge of Restate, workflow
  function names, internal projection tables, protobuf field names, or
  scenario fixture implementation details.
- Any usability hard gate scored `0` creates a follow-up with the failing
  surface, expected behavior, observed evidence, and reproduction command.

## Deterministic Checks

Run these checks from a clean checkout or a known scenario branch after the
scenario implementation workstream has produced the target artifacts:

```sh
make build
for scenario in scenarios/e2e/*.json; do
  target/debug/coat scenario run --file "$scenario" --output-dir target/coat-scenarios
done
target/debug/coat scenario report --run-dir target/coat-scenarios/<scenario-id>
```

For each reviewed scenario, record:

- Scenario ID, command, exit code, start time, and bounded clock or seed.
- SPA screenshot or trace for goal overview, task graph, blocked or waiting
  state, evidence, and completion state.
- TUI capture or transcript for selected goal, task counts, blocked work, next
  action, and completion state.
- `report.json` plus a human-readable report or markdown summary under the
  scenario evidence directory.
- A rubric table with scores and direct artifact references.

The deterministic review fails when:

- SPA and TUI selected-goal IDs or titles disagree for the same scenario.
- A blocked or waiting task lacks a human-readable reason and action path.
- A completion or failure state is only visible through raw JSON.
- The next action is ambiguous when operator action is required.
- Evidence exists only as an opaque payload with no task, command, review,
  artifact, or checkpoint explanation.
- Primary screenshots or reports expose implementation-only labels such as
  `GoalWorkflow/`, `workflow_compute_graph`, `payload_json`,
  `AgentRunResult`, or raw protobuf field names as the only way to understand
  the state.

## SPA Expectations

- The first goal view identifies the active goal, objective, done criteria,
  progress state, and current next action without requiring navigation into
  developer diagnostics.
- The task graph uses stable human-readable labels for task role, purpose,
  status, owner or runner class, blocked reason, and evidence availability.
- Waiting states separate human approval, human input, external callback,
  timer, capacity, model availability, and failure recovery.
- Evidence links or drawers explain what artifact is being shown and why it
  matters to goal satisfaction.
- Completion banners or summaries explain why the goal is complete, failed,
  cancelled, budget exhausted, or still incomplete.
- Developer details may be available behind explicit diagnostics, but they
  cannot be the primary comprehension path.

## TUI Expectations

- The dashboard shows the selected goal and the same goal context used by chat,
  task graph, human queue, and evidence commands.
- Keyboard navigation exposes a current task focus, blocked or waiting work,
  and next action without making the operator paste IDs or inspect JSON.
- TUI status words match the SPA status words closely enough that screenshots
  or transcripts can be compared mechanically.
- Goal draft submission, selected-goal changes, blocked human input, and
  completion status echo concise summaries in the chat log or dashboard.
- When the TUI cannot render a rich graph, it provides a readable outline of
  parent/child relationships and status counts.

## Execution Checklist

1. Pick the scenario run that represents the product path under review and
   record its command, run directory, and expected operator story.
2. Generate or collect SPA evidence for goal orientation, task graph, blocked
   state, next action, evidence, and completion status.
3. Generate or collect TUI evidence for the same selected goal and status
   state.
4. Produce the scenario report and confirm it answers the six operator
   questions in the acceptance criteria.
5. Score the rubric, linking each score to a screenshot, trace, transcript,
   command artifact, or report section.
6. File follow-ups for every hard-gate miss, including exact surface, expected
   operator-visible behavior, observed artifact, and reproduction command.
7. Re-run the scenario after fixes and preserve the before/after evidence in
   the scenario artifact tree or linked PR notes.

## Validation

- Run `git diff --check -- docs/exec-plans/completed/170-usability-coherence-evaluation.md docs/README.md docs/operations/cli.md` after edits to this plan or its references.
- Run markdown or link checks when a docs-wide checker is introduced.
- Do not treat this plan as complete until scenario evidence has been scored
  against the hard gates above.

## Follow-Ups

None currently. Residual UIE2E work is tracked by the master runtime plan.
