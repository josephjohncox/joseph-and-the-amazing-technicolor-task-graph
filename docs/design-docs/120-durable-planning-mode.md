# Durable Planning Mode

## Purpose

COAT should support the same planning workflow humans expect from modern chat systems, but the plan must be durable, typed, revisable, reviewable, and compilable into executable goal state.

Planning mode is for shaping work before agents start executing it. It is not an autonomous worker loop and not an alternate coordinator.

## Contract

The durable planning artifact is `DurablePlan`.

It contains:

- title, objective, repo, mode, and status;
- the original operator prompt;
- versioned `PlanRevision` records;
- current `GoalAuthoringGuidance`;
- current `GoalPlan` with stable subgoal IDs;
- proposed `initial_tasks`;
- open or answered `PlanQuestion` records;
- explicit `PlanDecision` records;
- optional compiled `GoalSpec` identity and quality report.

Supported statuses:

- `draft`;
- `needs_questions`;
- `ready_for_review`;
- `approved`;
- `compiled`;
- `superseded`;
- `archived`.

Supported modes:

- `interactive`;
- `autonomous`;
- `human_steered`;
- `research_first`;
- `implementation_ready`.

## Workflow

1. Draft a plan from a rough operator request.
2. Revise the plan as the conversation clarifies objective, evidence, constraints, questions, subgoals, and first tasks.
3. Review the plan as a plan, not as implementation.
4. Compile the plan into a `GoalSpec`.
5. Lint or submit the compiled goal.

Compiling does not submit the goal. Submission remains an explicit operator or coordinator action.

## Backend Surface

`coat-goal-store` stores plans through the same local JSONL or Postgres-backed projection path used for goals:

- `POST /goal-store/plans`;
- `GET /goal-store/plans`;
- `GET /goal-store/plans/{plan_id}`;
- `POST /goal-store/plans/{plan_id}/revisions`;
- `POST /goal-store/plans/{plan_id}/compile`.

Restate remains authoritative once a plan becomes a submitted goal. Before that point, the plan store is the durable planning workspace.

## CLI

Create a durable plan:

```sh
coat plan draft \
  --title "Durable planning mode" \
  --objective "Design and compile a plan into a typed GoalSpec before execution." \
  --prompt "Capture questions, decisions, subgoals, and first tasks before starting agents." \
  --mode interactive
```

List and inspect plans:

```sh
coat plan list
coat plan show --plan-id <plan-id>
```

Revise from JSON:

```sh
coat plan revise \
  --plan-id <plan-id> \
  --file examples/plan-revision-answer-questions.json
```

Compile into `GoalSpec`:

```sh
coat plan compile \
  --plan-id <plan-id> \
  --strict-review \
  --human-steered \
  --out examples/drafts/compiled-goal.json
```

## SPA And MCP

The control gateway has a `Plans` tab. It can create plans, list plans, load a plan, post revisions, and compile a plan to a `GoalSpec`.

The same tab renders a continuity view for the loaded plan:

- next actions;
- open and authoring questions;
- stable subgoals;
- coordinator-owned initial task seeds;
- planning decisions;
- revision history.

The MCP dashboard surface exposes:

- `coat_plan_list`;
- `coat_plan_get`;
- `coat_plan_continuity`;
- `coat_plan_compile`.

Plan editing through the SPA or MCP still calls backend APIs. The browser does not own durable plan state.

## Rules

- Use durable plans for ambiguous, multi-step, high-risk, or collaborative requests.
- Keep open questions explicit instead of hiding them inside a prompt.
- Record decisions and rationale when steering changes direction.
- Keep stable subgoal IDs once other artifacts refer to them.
- Compile to `GoalSpec` only when the plan has enough evidence, constraints, and first-frontier routing.
- Do not ask workers to infer subgoals from plan prose after compilation.
