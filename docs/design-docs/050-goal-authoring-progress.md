# Goal Authoring, Progress, And Task Distribution

## Purpose

Goals should enter the durable system as clean work contracts, not as vague prompts. The coordinator needs stable subgoal IDs, immediate work seeds, and queryable progress so it can distribute work across runners and subagents without asking any worker to own the global plan.

## Contract Shape

`GoalSpec` carries three goal-authoring surfaces:

- `authoring`: operator intent, assumptions, open questions, constraints, acceptance evidence, and out-of-scope work.
- `plan.subgoals`: stable subgoal records with ID, title, owner, expected artifacts, acceptance evidence, dependencies, and tags.
- `initial_tasks`: known first-frontier `ChildTaskRequest`s with matching `subgoal_id`, role, budget, sandbox, execution profile, and done criteria.

`GoalState::new` creates the root planner task and materializes each `initial_tasks` entry as a child `TaskNode`. The root remains the global planner; seeded tasks become normal durable tasks that runner selection can dispatch immediately.

## Progress Model

`GoalProgress` summarizes:

- total, runnable, completed, blocked, failed, and waiting task counts;
- task counts by status;
- per-subgoal progress;
- runnable task IDs ordered by priority and depth;
- current satisfaction report;
- remaining root budget.

Subgoal progress is derived from task `subgoal_id` links, not from natural-language prompt matching.

## Task Query Model

`TaskQuery` is the stable distribution and dashboard interface. It can filter by:

- `subgoal_id`;
- task status;
- worker role;
- task purpose kind;
- tag;
- runnable-only frontier;
- limit.

`TaskList` returns matching `TaskProgress` records plus the same `GoalProgress` snapshot, so a coordinator, UI, or runner queue can make local decisions with global context.

## Operator Flow

1. Draft the goal with the authoring loop in `docs/operations/goal-authoring.md`.
2. Run `coat goal lint --file <goal.json> --strict`.
3. Submit the goal.
4. Inspect durable status with `coat goal progress`.
5. Find distributable subgoal work with `coat goal tasks`.
6. Apply changes through `coat goal steer`, not by editing workflow state.

## Design Rules

- Workers may request children, but only the coordinator creates child task nodes.
- Workers should receive only their task, scoped memory context, MCP references, and relevant subgoal metadata.
- Subgoal IDs must be stable and human-readable enough for dashboards and notifications.
- A task without a subgoal is allowed only for root planning, global review, unification, or operator-injected emergency work.
- Progress must be calculated from durable state and validation reports, not sidecar-local thread state.
