# Review Doctrine Standard Library

COAT treats quality goals as typed policy, not reviewer folklore. A goal can opt in to `review_policy.doctrine`, select standard presets, add custom objectives, override built-ins, and require reviewer/tester/formal-methods workers to return structured coverage.

## Contract Shape

`ReviewDoctrine` has:

- `enabled`: explicit opt-in switch.
- `presets`: built-in libraries to expand.
- `coverage`: validation behavior for objective and gate results.
- `custom_objectives`: project or goal-specific review goals.
- `custom_evidence_requirements`: concrete evidence the reviewer should cite.
- `custom_style_doctrines`: style and design doctrines such as clean code, DDD, functional DDD, or virtuous laziness.
- `custom_validation_gates`: machine-checkable or reviewer-checkable gates.
- `custom_subagents`: reviewer, tester, research, or formal-methods personas that inherit the doctrine.
- `overrides`: disable, require, make optional, or set score thresholds for built-in or custom entries.

The standard library is available by preset, but it is not forced on every goal. Existing lightweight goals can keep doctrine disabled; strict goals enable it and decide how much coverage to require.

## Built-In Presets

- `core_engineering`: correctness, maintainability, abstraction quality, compile evidence.
- `testing`: regression tests, behavioral/end-to-end objective tests, and hypothesis/property/fuzz-style invariant tests.
- `formal_methods`: type soundness and formal verification posture.
- `functional_domain_driven_design`: DDD language, functional core, and denotational clarity.
- `laziness_lost`: simplicity, negative code, and abstraction that reduces future cognitive load.
- `security`: explicit security boundaries and least privilege.
- `performance`: benchmark or complexity evidence when performance matters.

The `laziness_lost` preset links to Bryan Cantrill's "The peril of laziness lost" as a style doctrine: LLM output should make systems smaller, clearer, and easier for future humans to change, not larger for vanity metrics.

## Standard Subagents

Presets expand to reusable subagent profiles:

- `subagent.code_reviewer`: reviewer role for correctness, maintainability, and abstraction.
- `subagent.tester`: tester role for regression, behavioral/end-to-end, and hypothesis/property testing.
- `subagent.formal_methods`: formal-methods role for type soundness, proof posture, and model-check questions.
- `subagent.functional_ddd`: reviewer role for DDD, functional core, and denotational semantics.
- `subagent.simplicity_critic`: reviewer role for generated-bulk and abstraction-debt checks.
- `subagent.library_researcher`: custom example profile for library and paper research.

Workers inherit the doctrine through `TaskNode.review_doctrine`. Review, test, validator, patch-merger, and formal-methods roles are expected to return coverage when `coverage.require_objective_results` or `coverage.require_gate_results` is enabled.

## Validation

`ReviewOutput` includes:

- `objective_results`: one result per required objective.
- `gate_results`: one result per required validation gate.

When doctrine coverage is required, `ValidationReport::from_result` blocks review validation if:

- a required objective is missing;
- an objective failed, was not checked, or is below threshold;
- required objective evidence is missing;
- a required gate is missing;
- a blocking gate failed or is below threshold;
- required gate evidence is missing.

This lets strict goals require compile, test, type-soundness, style, and formal-methods evidence without embedding that logic in sidecar-specific prompts.

## Behavioral Testing Doctrine

The `testing` preset is intentionally deeper than "a test command ran" or "the button/API exists." It includes:

- `testing.regression`: changed behavior has focused regression coverage.
- `testing.behavioral_end_to_end`: tests exercise the actual end-to-end objective, observable behavior, user/operator workflow, state transition, and meaningful failure modes.
- `testing.hypothesis`: important invariants have property, generative, fuzz, or explicit hypothesis-style tests when appropriate.

The required evidence includes `evidence.behavioral_scenarios`, and strict doctrine adds `gate.behavioral_coverage`. Reviewers and tester agents should reject shallow existence checks when the goal is behavioral. Examples of shallow checks that do not satisfy the doctrine:

- only checking that a button renders;
- only checking that an endpoint returns 200;
- only checking that a JSON schema file exists;
- only checking that a CLI command prints something;
- only checking that a route or config key is present.

Instead, tests should be falsifiable against the objective: they should fail when the key workflow is broken, when the state transition is wrong, when persistence/dedupe/routing is incorrect, when edge cases regress, or when the operator would receive misleading evidence.

For operator UI and workflow changes, the deterministic scenario gate is the
preferred behavioral evidence. PR CI runs every spec under `scenarios/e2e` with
`coat scenario run --file <scenario> --output-dir target/coat-scenarios`.
Those specs use stubbed workers, fixed seeds, bounded clocks, and local
fixtures so the result is reproducible. Each
`target/coat-scenarios/<scenario-id>` directory is the review artifact for
`evidence.behavioral_scenarios`; Playwright traces and screenshots are attached
on failure when the scenario drives the SPA.

Scenario tests should assert workflow semantics rather than static surface
presence. For the control gateway, that means proving the current-goal selector
or TUI selection flows into chat sessions, graph reads, control actions, memory
views, and human-queue state through backend APIs. A scenario that only proves a
page rendered or a button existed does not satisfy `gate.behavioral_coverage`.

## Standard Steering Checks

Operators can inject doctrine-backed checks into active goals through `SteeringDirectiveKind::RequestStandardReview`:

- `abstraction`
- `readability`
- `compile`
- `test_evidence`
- `behavioral_testing`
- `hypothesis_testing`
- `type_soundness`
- `formal_verification`
- `clean_code`
- `ddd`
- `functional_ddd`
- `denotational_semantics`
- `canonical_style`
- `library_fit`
- `reference_search`
- `web_search`
- `deep_research`
- `simplicity`
- `security`
- `output_safety`

Review-like checks create reviewer, tester, or formal-methods tasks. Research-like checks create sourced research tasks that must return source capture and an information-use plan. The coordinator still owns the task tree; steering only requests bounded work.

Use `behavioral_testing` when a goal needs a tester agent to inspect whether coverage proves the end-to-end objective:

```sh
coat goal steer-standard --goal-id <goal-id> --check behavioral_testing --topic "control gateway goal authoring"
```

`security` and `output_safety` are the standard executor guardrail checks. They are useful when a worker ran code, touched secrets, used open network, changed dependencies, returned large logs, or produced output that another agent might be tempted to follow as instructions.

Operators can submit these checks without hand-authoring JSON:

```sh
coat goal review-checks
coat goal steer-standard --goal-id <goal-id> --check abstraction --topic "coordinator task graph"
coat goal steer-standard --goal-id <goal-id> --check deep_research --topic "standard vector memory and RAG services"
```

Use `--emit-only` or `--out <file>` when a human should review the directive JSON before it is posted to Restate.

## Extension Rules

- Prefer a standard preset when it matches the intent.
- Add custom objectives for repo-specific doctrine or project-specific acceptance criteria.
- Add custom evidence requirements when a gate needs commands, artifacts, references, or proofs.
- Add custom subagents when a recurring reviewer persona deserves its own role, capabilities, or model route.
- Use overrides for local policy differences instead of forking the preset.
- Keep `coverage.require_*` false for exploratory goals and true for production-quality gates.

See `examples/goal-review-doctrine.json`, `examples/review-output-doctrine-coverage.json`, `examples/steering-standard-abstraction.json`, `examples/steering-standard-behavioral-testing.json`, and `examples/steering-standard-deep-research.json`.
