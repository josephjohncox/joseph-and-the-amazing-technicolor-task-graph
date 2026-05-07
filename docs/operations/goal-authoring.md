# Goal Authoring Guide

Good goals are executable contracts. They tell the coordinator what success means, what evidence is required, which workers may run, which facts need research, what memory should be reused, and where humans must stay in control.

Use this guide before `coat goal submit`.

## Goal Quality Bar

A submit-ready goal has:

- a concrete objective with a clear artifact or state change;
- stable subgoal IDs that split the goal into coordinator-visible slices of work;
- stable graph color keys for major workstreams when the goal needs visible task topology;
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

## Durable Planning Mode

Use `coat plan` when the request needs the normal chat-style planning phase before it should become an executable goal. A durable plan is versioned and can hold open questions, answers, decisions, subgoals, and proposed initial tasks.

Planning mode is appropriate when:

- the operator request is ambiguous;
- multiple subgoals or agents need coordination;
- research or memory preflight should happen before execution;
- the human wants to steer the shape of the work before runners receive tasks;
- the plan should be visible in the SPA or through MCP before it becomes a `GoalSpec`.

Create a plan:

```sh
coat plan draft --file examples/plan-draft-durable-mode.json
```

Revise it after questions are answered:

```sh
coat plan revise \
  --plan-id <plan-id> \
  --file examples/plan-revision-answer-questions.json
```

Compile it into a `GoalSpec` without submitting:

```sh
coat plan compile \
  --plan-id <plan-id> \
  --strict-review \
  --human-steered \
  --out examples/drafts/compiled-goal.json
```

Then lint and submit the compiled goal explicitly. Do not treat a planning-mode transcript as worker instructions after compilation; transfer the durable parts into `GoalSpec.authoring`, `GoalSpec.plan`, and `GoalSpec.initial_tasks`.

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

`id`: Optional for normal authoring. Omit it unless you intentionally need a
deterministic/idempotent workflow key. `coat goal submit` assigns an ID when the
field is missing and prints `goal_id`, `workflow_url`, and an
`export COAT_GOAL_ID=...` helper for follow-up commands.

`objective`: Write this as the contract a reviewer will judge. Avoid implementation-only phrasing unless the implementation itself is the artifact.

`authoring`: Record the operator intent, assumptions, open questions, constraints, acceptance evidence, and out-of-scope work. This is for goal quality and reviewability, not for hidden worker instructions.

`plan`: Define stable `subgoals` with IDs, titles, owners, expected artifacts, acceptance evidence, and dependencies. Coordinators and dashboards should use these IDs to group progress and find work.

`color_policy`: Keep the default technicolor purpose palette for most goals. Use `assignment_mode = purpose` when colors should follow work/research/review/validation/unification semantics, `status` when operators mainly need state-oriented dashboards, or `custom` when subgoals and child tasks carry explicit colors. Color keys are durable semantic labels; `hex` is only a display hint.

`plan.subgoals[].color`: Set this when a subgoal represents a distinct workstream that should stay visually stable across child tasks, branch candidates, reviewer tasks, notifications, and dashboard views. Use keys like `research_green`, `implementation_blue`, `review_purple`, or goal-specific keys such as `parser_gold`.

`initial_tasks`: Add only known first-frontier tasks. Set `title`, `role`, `subgoal_id`, `priority`, `tags`, `done_criteria`, `budget`, `sandbox`, and `execution` enough for a runner to pick up the work without reading the entire goal prose.

`initial_tasks[].color`: Omit this when the task should inherit its subgoal color. Set it only when a seeded task intentionally differs from the subgoal, such as a red-team review task inside an implementation subgoal.

`execution.subagents`: Leave this at the default for almost every goal. In COAT, "subagent" means a durable child task created by the coordinator and routed through the runner registry. Do not write goals that ask Codex, Claude Code, an SDK harness, or an MCP client to spawn its own hidden subagents. Workers request more help by returning `ChildTaskRequest` values in `AgentRunResult.child_requests`.

`done_criteria`: Set `tests_pass = true` when code paths change. Set `artifact_exists = true` for reports, plans, PRs, generated schemas, or deployment manifests. Use `validator_score_min` for review-quality thresholds.

`review_policy`: Keep enabled for any task that changes code, deployment, policy, memory promotion, or user-visible behavior. Use at least one critic and a unifier when multiple branches or agents contribute.

`review_policy.doctrine`: Opt in when the goal needs overarching review standards. Select presets such as `core_engineering`, `testing`, `formal_methods`, `functional_domain_driven_design`, `laziness_lost`, `security`, or `performance`; add custom objectives, evidence requirements, style doctrines, validation gates, subagent profiles, and overrides. Set `coverage.require_objective_results` and `coverage.require_gate_results` when reviewer/tester/formal-methods tasks must prove coverage before validation passes. See `examples/goal-review-doctrine.json`.

Standard review steering: Use `request_standard_review` to inject bounded checks while a goal is running. `abstraction`, `readability`, `clean_code`, `ddd`, `functional_ddd`, `denotational_semantics`, `canonical_style`, and `simplicity` spawn review-like tasks. `compile`, `test_evidence`, and `hypothesis_testing` spawn tester-style tasks. `type_soundness` and `formal_verification` spawn formal-methods tasks. `library_fit`, `reference_search`, `web_search`, and `deep_research` spawn sourced research tasks that return an information-use plan.

`control_policy`: Use `human_steered_continuous` for long-running initiatives that need operator steering. Use explicit stop conditions; do not rely on a free-running loop.

`restart_policy`: Enable for work that may be resumed after runner loss, timeouts, config repair, model changes, or operator steering. Keep max restart counts finite. Use `scope = task` for a single failed task, `scope = blocked` for all blocked tasks, and `scope = goal` only when the whole frontier should be requeued.

`timeout_policy`: Set task and runner-call timeouts so sidecars cannot hold a durable frontier forever. The coordinator records timed-out tasks as structured worker results, then applies `on_task_timeout`; the default is `restart_if_allowed`.

`branching_policy`: Enable when two or more implementations should compete. Use branch groups for high-risk code paths, model bakeoffs, ambiguous designs, or places where you want different personas to solve the same subgoal. Candidate branches should return artifacts; vote tasks return `BranchVoteOutput`; optional unifier tasks turn votes into one selected implementation.

`event_sources`: For recurring or real-world-triggered work, author an event source and route instead of asking an agent to sleep, poll, or watch forever. Event routes can create a new goal, create a research goal, steer an existing goal, or pause for human review. Schedule and webhook activation should go through approval when it introduces external callbacks, calendar access, or recurring spend.

`research_policy`: Enable when answers may change or when external claims affect implementation. Require sources and use plans.

`memory_policy`: Use goal, task, repo, and persona scopes by default. Branch workers may write branch-scoped memories; reviewer/unifier tasks decide what becomes shared.

`approval_policy`: Keep the default unless this is a trusted offline smoke test. Approval-policy `never` is acceptable only inside isolated runners with constrained filesystem and network.

`default_execution`: Pick the least powerful runner that can do the work. Add local model candidates when the runner can actually serve them. Add MCP servers by reference and keep auth in `SecretRef`. For Codex or Claude device/browser login, set `auth_distribution.mode = runner_local_only` and require runner labels; for distributed human auth, use a brokered lease and approval. For untrusted executor work, enable `guardrails` so output and security review tasks fork from completed work before goal satisfaction. Use `sandbox.network = restricted` by default, and set `isolation.egress_policy_ref`, `isolation.ingress_policy_ref`, and `isolation.network_policy_labels` when a Kubernetes, Cilium, Calico, cloud firewall, or provider sandbox policy should constrain the runner's blast radius.

`sandbox`: Use `isolation.backend` and runner labels to request the actual execution boundary. `local_workspace` is for trusted development. Use `container`, `gvisor`, `kata`, `firecracker`, `kubernetes_job`, or `provider_sandbox` for production execution when a runner can return an enforced `SandboxAttestation`. If `approval_policy = never`, require strong sandbox attestation.

## Submission Commands

Draft a starter goal without hand-writing the full contract:

```sh
coat goal draft \
  --title "Typed memory retrieval review" \
  --objective "Review and implement typed memory retrieval for runner-distributed tasks. Success means schemas regenerate, cargo tests pass, docs explain operator use, and reviewer doctrine accepts the evidence." \
  --strict-review \
  --human-steered \
  --subgoal "id=research-memory,title=Research memory substrate,role=research,objective=Find current supported memory/vector/RAG libraries,tags=research|memory" \
  --initial-task "role=research,subgoal=research-memory,title=Research memory substrate,prompt=Find current supported memory/vector/RAG libraries and return sourced recommendations,tags=research|preflight" \
  --out examples/drafts/typed-memory-retrieval.json
```

`--subgoal` and `--initial-task` accept comma-separated `key=value` fields. Use `|` inside list fields such as `tags`, `dependencies`, and `acceptance_evidence`. Color fields are `color`, `color_label`, `color_hex`, and `color_meaning`; omit task color when the task should inherit its subgoal color. For anything more complex than a seed frontier, draft JSON and then run the critic pass.

Example technicolor seed:

```sh
coat goal draft \
  --title "Technicolor graph smoke" \
  --objective "Show semantic graph colors across research, implementation, and review." \
  --subgoal "id=implement,title=Implement,objective=Make the contract real,role=codex,color=implementation_blue,color_label=Implementation Blue,color_hex=#2563eb" \
  --initial-task "role=codex,title=Implement,prompt=Add the contract,subgoal_id=implement"
```

List built-in standard checks:

```sh
coat goal review-checks
```

Lint before submit:

```sh
coat goal lint --file examples/goal-clean-plan.json --strict
```

Submit from JSON:

```sh
coat goal submit --file examples/goal-template-structured.json
export COAT_GOAL_ID=<goal-id-from-submit-output>
```

List projected goals or select the latest projected goal:

```sh
coat goal list
coat goal progress --latest
```

Check status:

```sh
coat goal status
```

Check progress:

```sh
coat goal progress
```

Find subgoal tasks:

```sh
coat goal tasks \
  --file examples/task-query-subgoal.json
coat goal tasks \
  --subgoal-id implement-progress-contract \
  --color implementation_blue \
  --runnable
```

Steer with research:

```sh
coat goal steer \
  --file examples/steering-request-research.json
```

Steer with a standard check directly:

```sh
coat goal steer-standard \
  --check deep_research \
  --topic "memory substrate and vector RAG libraries" \
  --reason "Implementation should be guided by current standard libraries and supported services."
```

Approve a waiting task:

```sh
coat approve \
  --approval-id <approval-request-id> \
  --approved true
```

Follow-up commands resolve the goal from `--goal-id`, `COAT_GOAL_ID`, or
`--latest`. Goal-scoped JSON files for `steer`, `restart`, `branch`, and
`select-branch` may omit `goal_id`; the CLI injects the selected workflow key
and rejects the command if the file contains a different `goal_id`.
