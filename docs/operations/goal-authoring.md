# Goal Authoring Guide

Good goals are executable contracts. They tell the coordinator what success means, what evidence is required, which workers may run, which facts need research, what memory should be reused, and where humans must stay in control.

Use this guide before `coat goal submit`.

## Goal Quality Bar

A submit-ready goal has:

- a concrete objective with a clear artifact or state change;
- stable subgoal IDs that split the goal into coordinator-visible slices of work;
- initial tasks linked to those subgoal IDs when the first frontier is already known;
- done criteria that can be validated by tests, artifacts, reviewer scores, or explicit evidence;
- constraints that should not be violated, including repo scope, safety, compliance, latency, budget, and model/provider limits;
- a research policy when facts may be stale, niche, external, or high-stakes;
- a memory policy that states what shared context to retrieve and how branch memories are promoted;
- an execution profile that names runner capability, model route, persona, MCP context, and notification targets;
- an approval policy for risky operations such as open network, secrets, non-isolated runners, dangerous MCP tools, deploys, merges, or tracker state changes.

Weak goal:

```text
Make the agent system better.
```

Usable goal:

```text
Implement approval-gated distributed memory retrieval for coding and review workers. Success means schemas regenerate, local Compose config validates, cargo tests pass, docs explain the operator workflow, and the reviewer task accepts the evidence.
```

## Authoring Loop

Run goal authoring as a short actor/critic loop before submitting anything durable.

1. Intake pass

   Ask what outcome is needed, why it matters, what should not change, what evidence proves completion, and which systems or repos are in scope.

2. Memory preflight

   Search shared memory for prior decisions, repo rules, failed attempts, approvals, model routing preferences, and known constraints. Use facts only when they have source, scope, timestamp, and provenance.

3. Research preflight

   If the goal depends on current product behavior, third-party APIs, package support, prices, laws, security posture, or model capabilities, create a research task before implementation. Research output must include sources, confidence, open questions, and an `InformationUsePlan`.

4. GoalSpec draft

   Convert the intake into `GoalSpec` JSON. Keep the objective human-readable, but make the policies machine-readable. Add `authoring` notes for the human reasoning trail, `plan.subgoals` for durable work slices, and `initial_tasks` with matching `subgoal_id`s for any work the coordinator can dispatch immediately.

5. Critic pass

   Review the draft for vague success criteria, missing budget, overbroad filesystem/network access, missing approval gates, unbounded child spawning, missing memory scopes, and unsupported runner/model assumptions.

6. Schema and smoke pass

   Validate that the JSON parses as `GoalSpec`, regenerate schemas after contract changes, then submit to a local stub stack before enabling live workers.

7. Steering loop

   Use `coat goal steer` for changes after submission. Do not edit durable state by hand. Steering can add constraints, update the objective, inject bounded tasks, request research, pause, resume, or cancel.

## Copyable LLM Prompts

Goal intake prompt:

```text
You are the COAT goal intake agent. Turn the operator request into a bounded engineering goal.

Ask only the missing questions needed to define:
- objective
- artifacts
- acceptance evidence
- out-of-scope changes
- approval risks
- required research
- memory context to retrieve
- preferred runners/models
- budget and stop conditions

If the request is already clear, do not ask questions. Produce a concise intake summary and a list of unresolved assumptions.
```

Memory preflight prompt:

```text
You are the COAT memory preflight agent.

Given a draft goal, search durable memory for relevant prior decisions, repo rules, implementation attempts, approvals, failures, and user preferences.

Return:
- facts_to_use
- facts_to_avoid
- memory_keys
- confidence
- missing_context
- proposed GoalSpec changes

Do not promote branch memory. Promotion is reserved for reviewer or unifier tasks.
```

GoalSpec compiler prompt:

```text
You are the COAT GoalSpec compiler.

Convert the intake summary, memory preflight, and research preflight into valid GoalSpec JSON.

Rules:
- keep the objective concrete and testable;
- include authoring guidance, plan subgoals, and initial task routing when the first frontier is known;
- include control_policy, research_policy, memory_policy, approval_policy, and default_execution;
- use SecretRef for auth, never raw tokens;
- use auth_distribution for device sessions, workload identity, brokered user auth, and required runner labels;
- use bounded budgets;
- require review and unification for non-trivial work;
- prefer local stub-safe execution unless the operator explicitly asks for live workers;
- include initial_tasks only when dependencies are already known.

Return JSON only.
```

Critic prompt:

```text
You are the COAT goal critic.

Review the GoalSpec for execution risk and ambiguity. Block submission if any of these are true:
- success cannot be validated;
- runner/model assumptions are unsupported;
- approval gates are missing for dangerous work;
- memory writes can pollute shared context before review;
- research is needed but disabled;
- budgets allow unbounded recursion;
- MCP auth embeds raw credentials;
- device/browser auth is copied between nodes instead of runner-local or brokered;
- the goal asks a worker to own the global plan.

Return decision: accept, changes_requested, blocked, or inconclusive.
Include concrete edits when changes are requested.
```

## Field Guidance

`objective`: Write this as the contract a reviewer will judge. Avoid implementation-only phrasing unless the implementation itself is the artifact.

`authoring`: Record the operator intent, assumptions, open questions, constraints, acceptance evidence, and out-of-scope work. This is for goal quality and reviewability, not for hidden worker instructions.

`plan`: Define stable `subgoals` with IDs, titles, owners, expected artifacts, acceptance evidence, and dependencies. Coordinators and dashboards should use these IDs to group progress and find work.

`initial_tasks`: Add only known first-frontier tasks. Set `title`, `role`, `subgoal_id`, `priority`, `tags`, `done_criteria`, `budget`, `sandbox`, and `execution` enough for a runner to pick up the work without reading the entire goal prose.

`done_criteria`: Set `tests_pass = true` when code paths change. Set `artifact_exists = true` for reports, plans, PRs, generated schemas, or deployment manifests. Use `validator_score_min` for review-quality thresholds.

`review_policy`: Keep enabled for any task that changes code, deployment, policy, memory promotion, or user-visible behavior. Use at least one critic and a unifier when multiple branches or agents contribute.

`control_policy`: Use `human_steered_continuous` for long-running initiatives that need operator steering. Use explicit stop conditions; do not rely on a free-running loop.

`event_sources`: For recurring or real-world-triggered work, author an event source and route instead of asking an agent to sleep, poll, or watch forever. Event routes can create a new goal, create a research goal, steer an existing goal, or pause for human review. Schedule and webhook activation should go through approval when it introduces external callbacks, calendar access, or recurring spend.

`research_policy`: Enable when answers may change or when external claims affect implementation. Require sources and use plans.

`memory_policy`: Use goal, task, repo, and persona scopes by default. Branch workers may write branch-scoped memories; reviewer/unifier tasks decide what becomes shared.

`approval_policy`: Keep the default unless this is a trusted offline smoke test. Approval-policy `never` is acceptable only inside isolated runners with constrained filesystem and network.

`default_execution`: Pick the least powerful runner that can do the work. Add local model candidates when the runner can actually serve them. Add MCP servers by reference and keep auth in `SecretRef`. For Codex or Claude device/browser login, set `auth_distribution.mode = runner_local_only` and require runner labels; for distributed human auth, use a brokered lease and approval.

## Submission Commands

Lint before submit:

```sh
cargo run -p coat-cli -- goal lint --file examples/goal-clean-plan.json --strict
```

Submit from JSON:

```sh
cargo run -p coat-cli -- goal submit --file examples/goal-template-structured.json
```

Check status:

```sh
cargo run -p coat-cli -- goal status --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
```

Check progress:

```sh
cargo run -p coat-cli -- goal progress --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611
```

Find subgoal tasks:

```sh
cargo run -p coat-cli -- goal tasks \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611 \
  --file examples/task-query-subgoal.json
```

Steer with research:

```sh
cargo run -p coat-cli -- goal steer \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611 \
  --file examples/steering-request-research.json
```

Approve a waiting task:

```sh
cargo run -p coat-cli -- approve \
  --goal-id 018f8f2f-1fd8-7688-bb12-8bfb6b756611 \
  --approval-id <approval-request-id> \
  --approved true
```
