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

   If the work needs local binaries such as `git`, `docker`, `helm`, `kubectl`,
   package managers, build tools, or project CLIs, declare them in
   `default_execution.local_tools` or the specific child task execution profile.
   Add the matching runner capabilities and labels. Do not rely on prompt text
   like "run whatever commands are needed"; local tool use must be structured,
   allowlisted, approval-aware, and produce command evidence artifacts.

5. Critic pass

   Review the draft for vague success criteria, missing budget, overbroad filesystem/network access, missing approval gates, unbounded child spawning, missing memory scopes, and unsupported runner/model assumptions.

6. Schema and smoke pass

   Validate that the JSON parses as `GoalSpec`, regenerate schemas after contract changes, then submit to a local stub stack before enabling live workers.

7. Steering loop

   Use `coat goal steer` for changes after submission. Do not edit durable state by hand. Steering can add constraints, update the objective, inject bounded tasks, request research, evaluate goal completion, expand or edit done criteria, pause, resume, or cancel.

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

Use plan branching when the operator wants competing approaches without
rewriting the original planning history. Create a new plan with
`source_plan_id`, revise that branch independently, and compile it with a new
`goal_id`:

```sh
coat plan draft --file examples/plan-branch-from-existing.json
coat plan revise \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756710 \
  --file examples/plan-revision-branch-local-runners.json
coat plan compile \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756710 \
  --file examples/plan-compile-branch-new-goal.json \
  --out examples/drafts/local-model-runner-branch-goal.json
```

Score and select branch candidates through the goal-store instead of editing plan
JSON directly. The source plan keeps the votes and selected candidate; the
candidate plan must have `source_plan_id` equal to the source plan ID, and
selection requires a compiled `goal_id` by default:

```sh
coat plan vote-candidate \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/plan-candidate-vote.json
coat plan select-candidate \
  --plan-id 018f8f2f-1fd8-7688-bb12-8bfb6b756700 \
  --file examples/plan-candidate-selection.json
```

## Adversarial Workflows

Use adversarial workflows when one answer is not enough: high-risk code,
ambiguous designs, safety-sensitive changes, model bakeoffs, or work that
benefits from independent disagreement.

Start from the shortcut command when possible:

```sh
coat goal adversarial plan \
  --goal-id <goal-id> \
  --actor-count 3 \
  --critic-check test_evidence \
  --critic-check security \
  --research-topic "current dependency and sandbox risks" \
  --unifier \
  --emit-only \
  --out-dir /tmp/coat-adversarial

coat goal adversarial start \
  --goal-id <goal-id> \
  --actor-count 3 \
  --critic-check test_evidence \
  --critic-check security \
  --min-satisfaction 0.9
```

Operational flow:

1. Seed an actor task for the intended artifact and keep the done criteria
   objective.
2. Add critic tasks through `review_policy`, `request_standard_review`, or
   explicit child tasks. Critics inspect artifacts, tests, scope, safety, and
   missing evidence; they do not rewrite the actor output silently.
3. Fork candidate branches with `branching_policy` when multiple approaches
   should compete. Candidates may use different roles, personas, model routes,
   sandbox profiles, or prompts, but each must return artifacts and evidence.
4. Add research tasks when external or current facts matter. Research returns
   sourced `ResearchOutput` and an `InformationUsePlan`; actors and critics use
   that plan instead of copying raw research into prompts.
5. Vote on candidates with `branch_vote` tasks or `coat goal branch` followed
   by `coat goal select-branch`. Votes must be structured `BranchVoteOutput`
   records with reasons, scores, and evidence references.
6. Join accepted work through a unifier. The unifier resolves conflicts,
   preserves the selected artifacts, promotes only reviewed branch memory, and
   records why losing branches were rejected.
7. Validate completion from durable task state, not from a worker claim. Use
   `evaluate_goal_completion` before adding more work or marking the goal done.

Personas are task-local. Use them to create useful disagreement such as
`implementer`, `security_critic`, `test_critic`, `formal_methods_reviewer`,
`operator_experience_reviewer`, `cost_reviewer`, or `researcher`. Do not infer
persona from role alone: a `codex` role can run a cautious maintainer persona,
and a reviewer role can run a product-risk persona. Keep personas short,
bounded, and tied to evidence they must produce.

Drill into task context before steering. Inspect the selected task's prompt,
execution profile, persona, MCP refs, memory context, result refs, child
requests, chat/session refs, command evidence, and reviewer output. If the
operator needs more context, request a research task, a standard review, or a
branch vote instead of asking an existing worker to improvise hidden work.

Recommended shortcut flows:

- `strict_review`: actor, tests, critic, and unifier for code or policy changes.
- `red_team`: actor plus security/safety critic before validation.
- `model_bakeoff`: fork two or more model/persona candidates, vote, then unify.
- `research_first`: research task, information-use plan, actor, critic.
- `test_first`: tester drafts failing evidence, actor fixes, critic verifies.
- `cheap_then_deep`: fast actor attempt, deep critic only when evidence is weak.
- `operator_review`: pause after critic output and require human branch
  selection before unification.

## Copyable LLM Prompts

Goal intake prompt:

```text
<coat_goal_intake>
  <role>You are the COAT goal intake agent.</role>
  <mission>Turn the operator request into a bounded engineering goal.</mission>
  <must>
    <rule>You MUST ask only questions needed to define objective, artifacts, acceptance evidence, out-of-scope changes, approval risks, required research, memory context, preferred runners/models, budget, priority/ranking intent, and stop conditions.</rule>
    <rule>You MUST NOT ask questions when the request is already clear enough to draft a bounded goal.</rule>
    <rule>You MUST produce a concise intake summary and unresolved assumptions.</rule>
    <rule>You MUST treat "subagent" as a COAT durable child task owned by the coordinator.</rule>
  </must>
  <output>Return structured JSON with keys: intake_summary, missing_questions, unresolved_assumptions, approval_risks, research_needs, memory_queries, runner_preferences, ranking_intent, stop_conditions.</output>
</coat_goal_intake>
```

Memory preflight prompt:

```text
<coat_memory_preflight>
  <role>You are the COAT memory preflight agent.</role>
  <input>Given a draft goal, search durable memory for relevant prior decisions, repo rules, implementation attempts, approvals, failures, and user preferences.</input>
  <must>
    <rule>You MUST return facts_to_use, facts_to_avoid, memory_keys, confidence, missing_context, and proposed_goal_spec_changes.</rule>
    <rule>You MUST preserve provenance for every durable fact.</rule>
    <rule>You MUST NOT promote branch memory.</rule>
    <rule>You MUST NOT treat unreviewed worker output as shared durable truth.</rule>
  </must>
  <reserved_authority>Memory promotion is reserved for reviewer or unifier tasks.</reserved_authority>
</coat_memory_preflight>
```

GoalSpec compiler prompt:

```text
<coat_goalspec_compiler>
  <role>You are the COAT GoalSpec compiler.</role>
  <input>Convert the intake summary, memory preflight, and research preflight into valid GoalSpec JSON.</input>
  <must>
    <rule>The objective MUST be concrete and testable.</rule>
    <rule>The GoalSpec MUST include authoring guidance, plan subgoals, and initial task routing when the first frontier is known.</rule>
    <rule>The GoalSpec MUST include control_policy, research_policy, memory_policy, approval_policy, and default_execution.</rule>
    <rule>The GoalSpec SHOULD include ranking_policy only when the operator wants explicit upvote/downvote promotion or demotion behavior.</rule>
    <rule>Auth MUST use SecretRef, auth_distribution, workload identity, runner-local device auth, or brokered user auth. Raw tokens MUST NOT appear.</rule>
    <rule>Budgets MUST be bounded.</rule>
    <rule>Non-trivial work MUST require review and unification evidence.</rule>
    <rule>Live workers MUST NOT be selected unless the operator explicitly asked for them.</rule>
    <rule>initial_tasks MUST appear only when dependencies are already known.</rule>
  </must>
  <output>Return JSON only.</output>
</coat_goalspec_compiler>
```

Critic prompt:

```text
<coat_goal_critic>
  <role>You are the COAT goal critic.</role>
  <mission>Review the GoalSpec for execution risk and ambiguity.</mission>
  <must_block_submission_when>
    <condition>Success cannot be validated.</condition>
    <condition>Runner or model assumptions are unsupported.</condition>
    <condition>Approval gates are missing for dangerous work.</condition>
    <condition>Memory writes can pollute shared context before review.</condition>
    <condition>Research is needed but disabled.</condition>
    <condition>Budgets allow unbounded recursion.</condition>
    <condition>MCP auth embeds raw credentials.</condition>
    <condition>Device or browser auth is copied between nodes instead of runner-local or brokered.</condition>
    <condition>The goal asks a worker to own the global plan.</condition>
  </must_block_submission_when>
  <must>
    <rule>You MUST return decision: accept, changes_requested, blocked, or inconclusive.</rule>
    <rule>You MUST include concrete edits when changes are requested.</rule>
    <rule>You MUST NOT rewrite the goal silently; proposed changes must be explicit.</rule>
  </must>
</coat_goal_critic>
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

`ranking_policy`: Leave disabled for ordinary one-off goals. Enable it when operators or the coordinator should upvote/downvote priority, promote a goal into an overarching initiative, or demote it under another goal as a subgoal. Votes are durable state on `GoalState`, and ranking decisions should change scheduling/projection behavior only through the coordinator.

`mechanism_policy`: Leave disabled unless the goal explicitly needs distributed consensus, voting, mechanism design, or auction behavior. Enable it when agents, humans, and the coordinator should participate in structured `MechanismRound`s for subgoal selection, branch selection, runner allocation, budget allocation, review-panel selection, or work auctions. Mechanism outcomes are durable recommendations until the coordinator applies them through normal ranking, branch, task, approval, or capacity state. Require human ratification for high-impact auctions or resource allocation.

`plan.subgoals[].color`: Set this when a subgoal represents a distinct workstream that should stay visually stable across child tasks, branch candidates, reviewer tasks, notifications, and dashboard views. Use keys like `research_green`, `implementation_blue`, `review_purple`, or goal-specific keys such as `parser_gold`.

`initial_tasks`: Add only known first-frontier tasks. Set `title`, `role`, `subgoal_id`, `priority`, `tags`, `done_criteria`, `budget`, `sandbox`, and `execution` enough for a runner to pick up the work without reading the entire goal prose.

`initial_tasks[].color`: Omit this when the task should inherit its subgoal color. Set it only when a seeded task intentionally differs from the subgoal, such as a red-team review task inside an implementation subgoal.

`execution.subagents`: Leave this at the default for almost every goal. In COAT, "subagent" means a durable child task created by the coordinator and routed through the runner registry. Do not write goals that ask Codex, Claude Code, an SDK harness, or an MCP client to spawn its own hidden subagents. Workers request more help by returning `ChildTaskRequest` values in `AgentRunResult.child_requests`.

`done_criteria`: Set `tests_pass = true` when code paths change. Set `artifact_exists = true` for reports, plans, PRs, generated schemas, or deployment manifests. Use `validator_score_min` for review-quality thresholds.

Criterion steering: Use `evaluate_goal_completion` when the coordinator should recompute satisfaction from durable task state before creating more tasks. Use `expand_done_criteria` when evidence requirements become stricter; it is monotonic and rejects boolean relaxations. Use `update_done_criteria` only when the original criterion was wrong and needs an explicit replacement. Set `apply_to_open_tasks` to push the new criterion into unfinished work, and set `reopen_terminal_tasks` only when completed work must be checked again under the new contract.

`review_policy`: Keep enabled for any task that changes code, deployment, policy, memory promotion, or user-visible behavior. Use at least one critic and a unifier when multiple branches or agents contribute.

`review_policy.doctrine`: Opt in when the goal needs overarching review standards. Select presets such as `core_engineering`, `testing`, `formal_methods`, `functional_domain_driven_design`, `laziness_lost`, `security`, or `performance`; add custom objectives, evidence requirements, style doctrines, validation gates, subagent profiles, and overrides. Set `coverage.require_objective_results` and `coverage.require_gate_results` when reviewer/tester/formal-methods tasks must prove coverage before validation passes. See `examples/goal-review-doctrine.json`.

Standard review steering: Use `request_standard_review` to inject bounded checks while a goal is running. `abstraction`, `readability`, `clean_code`, `ddd`, `functional_ddd`, `denotational_semantics`, `canonical_style`, and `simplicity` spawn review-like tasks. `compile`, `test_evidence`, and `hypothesis_testing` spawn tester-style tasks. `type_soundness` and `formal_verification` spawn formal-methods tasks. `library_fit`, `reference_search`, `web_search`, and `deep_research` spawn sourced research tasks that return an information-use plan.

`control_policy`: Use `human_steered_continuous` for long-running initiatives that need operator steering. Use explicit stop conditions; do not rely on a free-running loop.

`restart_policy`: Enable for work that may be resumed after runner loss, timeouts, config repair, model changes, or operator steering. Keep max restart counts finite. Use `scope = task` for one specific task, `scope = blocked` for all blocked tasks, `scope = failed` for failed tasks, `scope = timed_out` for timeout-backed work, and `scope = goal` only when the whole frontier should be requeued. Blocked, failed, waiting, and budget-exhausted states are recoverable; use `cancel` only when the goal should stop.

`timeout_policy`: Set task and runner-call timeouts so sidecars cannot hold a durable frontier forever. The coordinator records timed-out tasks as structured worker results, then applies `on_task_timeout`; the default is `restart_if_allowed`.

`branching_policy`: Enable when two or more implementations should compete. Use branch groups for high-risk code paths, model bakeoffs, ambiguous designs, or places where you want different personas to solve the same subgoal. Candidate branches should return artifacts; vote tasks return `BranchVoteOutput`; optional unifier tasks turn votes into one selected implementation.

`event_sources`: For recurring or real-world-triggered work, author an event source and route instead of asking an agent to sleep, poll, or watch forever. Event routes can create a new goal, create a research goal, steer an existing goal, or pause for human review. Schedule and webhook activation should go through approval when it introduces external callbacks, calendar access, or recurring spend.

Delayed compute: When work needs a human answer, approval, external callback, timer, resource, or model-route availability before it can continue, represent that pause as a `DelayedComputeThunk`. The thunk stores the wait reference and delimited continuation reference. A worker that discovers a wait returns `status = waiting` plus `AgentRunResult.delayed_compute_thunks`; the coordinator materializes the thunk, marks the task waiting, exposes it in `GoalProgress.compute_graph`, and resumes it only through a coordinator operation. Workers should not sleep, spin, or poll inside a task.

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

Steer the coordinator to evaluate or revise completion criteria:

```sh
coat goal steer \
  --file examples/steering-evaluate-goal-completion.json
coat goal steer \
  --file examples/steering-expand-done-criteria.json
coat goal steer \
  --file examples/steering-update-done-criteria.json
```

Steer with a standard check directly:

```sh
coat goal steer-standard \
  --check deep_research \
  --topic "memory substrate and vector RAG libraries" \
  --reason "Implementation should be guided by current standard libraries and supported services."
coat goal steer-standard \
  --check behavioral_testing \
  --topic "end-to-end objective and operator workflow" \
  --reason "Tests must prove the goal behavior and fail for meaningful incorrect implementations."
```

Vote on goal priority or hierarchy when `ranking_policy.enabled`:

```sh
coat goal vote \
  --direction up \
  --suggested-role overarching_goal \
  --reason "This is an umbrella objective for several active subgoals."
coat goal vote \
  --direction down \
  --suggested-role subgoal \
  --reason "This belongs under the platform-hardening initiative."
```

Start a mechanism-design round and submit a ballot when `mechanism_policy.enabled`:

```sh
coat goal mechanism start \
  --file examples/mechanism-round-consensus.json
coat goal mechanism ballot \
  --file examples/mechanism-ballot-consensus.json
```

Create and inspect a delayed compute thunk when an operator, event gateway, or
worker result needs to suspend a task at a delimited continuation:

```sh
coat goal thunk create \
  --file examples/delayed-compute-thunk-human-input.json
coat goal compute-graph
```

Approve a waiting task:

```sh
coat human approve \
  --approval-id <approval-request-id> \
  --approved true
```

Resume a delayed compute thunk after human input or an external answer arrives:

```sh
coat human resume-thunk \
  --thunk-id <thunk-id> \
  --response-summary "Use the local smoke runner before live worker execution."
```

Follow-up commands resolve the goal from `--goal-id`, `COAT_GOAL_ID`, or
`--latest`. Goal-scoped JSON files for `steer`, `restart`, `branch`, and
`select-branch` may omit `goal_id`; the CLI injects the selected workflow key
and rejects the command if the file contains a different `goal_id`.
