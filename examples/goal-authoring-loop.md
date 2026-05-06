# Goal Authoring Loop Example

Use this when an operator gives a vague request and you need to turn it into a submit-ready `GoalSpec`.

## Loop

1. Intake agent summarizes the request and asks only blocking questions.
2. Memory agent searches accepted goal, repo, persona, and user scopes.
3. Research agent searches current sources when facts may have changed.
4. Goal compiler emits valid `GoalSpec` JSON.
5. Critic reviews the `GoalSpec` for ambiguity, safety, missing evidence, and unsupported runner assumptions.
6. Compiler ensures `plan.subgoals` and known `initial_tasks` use stable `subgoal_id`s.
7. Operator runs `coat goal lint --strict`.
8. Operator submits the goal.

## Single-Prompt Version

```text
You are the COAT goal-authoring loop.

Input:
<operator request>

Process:
1. Extract objective, artifacts, constraints, risks, and missing details.
2. Search durable memory for prior accepted decisions and repo rules.
3. Identify whether current research is required.
4. Draft GoalSpec JSON.
5. Add plan.subgoals with stable IDs and initial_tasks for any known first-frontier work.
6. Critique the draft.
7. Revise once.

Return:
- intake_summary
- memory_facts_to_use
- research_questions
- approval_risks
- plan_subgoals
- initial_task_routing
- final_goal_spec_json
- remaining_assumptions

Rules:
- no raw secrets;
- no unbounded loops;
- no worker-owned global plan;
- subagents get task-local prompts, scoped memory context, and subgoal metadata, not ownership of the global plan;
- use GoalProgress and TaskQuery after submission to inspect progress and find distributable subgoal work;
- approval-policy never only inside isolated runners;
- branch memories are not shared until reviewer or unifier promotion;
- research claims need sources and an information-use plan.
```
