# Goal Authoring, Progress, And Task Distribution

## Purpose

Goals should enter the durable system as clean work contracts, not as vague prompts. The coordinator needs stable subgoal IDs, immediate work seeds, and queryable progress so it can distribute work across runners and subagents without asking any worker to own the global plan. The durable `goal_id` is required internally, but normal authors should not need to hand-write it.

## Contract Shape

`GoalSpec` carries three goal-authoring surfaces:

- `id`: optional in normal JSON authoring; generated on submit or deserialization unless the operator intentionally supplies a deterministic workflow key.
- `authoring`: operator intent, assumptions, open questions, constraints, acceptance evidence, and out-of-scope work.
- `plan.subgoals`: stable subgoal records with ID, title, owner, expected artifacts, acceptance evidence, dependencies, and tags.
- `initial_tasks`: known first-frontier `ChildTaskRequest`s with matching `subgoal_id`, role, budget, sandbox, execution profile, and done criteria.
- `color_policy`, `SubgoalSpec.color`, and `ChildTaskRequest.color`: durable semantic graph colors for the Technicolor Task Graph. Colors should communicate workstream meaning, such as research, implementation, review, validation, unification, or a custom branch group.
- `review_policy.doctrine`: optional standard or custom library of typed review objectives, evidence requirements, style doctrines, validation gates, subagent profiles, and overrides.
- `restart_policy`, `timeout_policy`, and `branching_policy`: operational controls for retrying goals, bounding runner calls, and splitting the same subgoal across multiple candidate implementations.

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
- graph color key;
- tag;
- runnable-only frontier;
- limit.

`TaskList` returns matching `TaskProgress` records plus the same `GoalProgress` snapshot, so a coordinator, UI, or runner queue can make local decisions with global context.

## Operator Flow

1. Draft the goal with the authoring loop in `docs/operations/goal-authoring.md`.
2. Use `coat goal draft` when the operator wants a starter `GoalSpec` with subgoals, initial tasks, strict review doctrine, human-steered mode, or branching enabled from CLI flags.
3. Run `coat goal lint --file <goal.json> --strict`.
4. Submit the goal and capture the printed `goal_id` with `export COAT_GOAL_ID=...`, or use `--latest` against the goal store for quick local workflows.
5. Inspect durable status with `coat goal progress`.
6. Find distributable subgoal work with `coat goal tasks`.
7. Branch risky or high-value work with `coat goal branch --file examples/branch-request-root.json`.
8. Select a winning branch with `coat goal select-branch` after vote/unifier evidence is available.
9. Restart blocked or timed-out work with `coat goal restart --file examples/restart-request-task.json`.
10. Inject standard checks with `coat goal steer-standard --check abstraction`, `coat goal steer-standard --check behavioral_testing`, `coat goal steer-standard --check deep_research`, or `coat goal steer --file examples/steering-standard-abstraction.json`.
11. Apply changes through steering commands, not by editing workflow state.

## Design Rules

- Workers may request children, but only the coordinator creates child task nodes.
- Goal prompts and reviewer doctrines should say "subagent" only when they mean a COAT durable child task. Do not ask a runner to use Codex, Claude Code, SDK, or MCP-native subagents directly.
- Workers should receive only their task, scoped memory context, MCP references, and relevant subgoal metadata.
- Subgoal IDs must be stable and human-readable enough for dashboards and notifications.
- Graph colors must use stable keys. UI hex values are presentation hints; the durable meaning is the color key plus `meaning`.
- A task without a subgoal is allowed only for root planning, global review, unification, or operator-injected emergency work.
- Progress must be calculated from durable state and validation reports, not sidecar-local thread state.
- Follow-up commands select workflows with `--goal-id`, `COAT_GOAL_ID`, or `--latest`; goal-scoped command JSON may omit `goal_id` when the CLI already selected the workflow.
- Branching is a durable task-tree operation: the original target task, candidate tasks, vote tasks, unifier task, and final selection are all queryable state.
- Timeouts and restarts are policy-controlled. A timed-out runner call may restart a task only if `RestartPolicy` allows the reason and scope.
- Doctrine coverage is a validation concern. If a goal requires objective or gate results, reviewer/tester/formal-methods outputs must return structured `ReviewOutput.objective_results` and `ReviewOutput.gate_results`.
