# 190 Adversarial Actor Critic Shortcuts

## Objective

Make actor/critic, adversarial review, model bakeoff, and branch-unification
workflows first-class operator shortcuts. The coordinator must still own durable
task truth: actors, critics, researchers, voters, and unifiers become durable
tasks, not native hidden subagents inside Codex, Claude Code, or a model runner.

## Status

Completed on 2026-05-12. Live provider execution remains governed by
`docs/exec-plans/active/160-live-durable-runtime-and-execution.md`; this plan
adds the typed shortcut surface, operator visibility, and gateway drill-down.

## Product Questions

- How does an operator start an actor/critic round without manually assembling
  branch, mechanism, research, and steering JSON?
- How can an operator inspect which persona, model route, runner, prompt, chat
  session, and external thread belongs to each actor or critic task?
- How can operators change personalities or model labels before starting an
  adversarial run?
- How are votes, reviews, unification, satisfaction, and evidence visible in
  the TUI and gateway with compact context refs?
- Which adjacent shortcut flows should become durable commands rather than
  informal prompt patterns?

## Decisions

- Add `coat goal adversarial plan` and `coat goal adversarial start`.
- `plan` writes the typed request bundle locally for review.
- `start` writes the same bundle, then posts branch, mechanism, and steering
  requests through existing durable workflow endpoints unless `--emit-only` is
  set.
- Require at least two actors so adversarial runs are actually comparative.
- Personas and model labels are task-local inputs to the shortcut; they do not
  redefine global runner roles.
- Agent-to-agent context is exposed by gateway projection and MCP through
  `coat_operator_agent_context`, using prompt/session/thread refs and compact task
  metadata.
- The TUI gets a dedicated Adversarial tab for satisfaction, actor candidates,
  critic checks, research, unification, votes, and context refs.
- Similar shortcuts should be built from the same typed pieces: strict review,
  red-team review, model bakeoff, research-first, test-first, cheap-then-deep,
  and operator review.

## Implementation Workstreams

- CLI shortcut: build branch, mechanism, and steering requests from one command
  while preserving goal/task/subgoal selectors and durable workflow posting.
- TUI drill-down: add an Adversarial dashboard tab and task grouping for actors,
  critics, research, unifiers, mechanism rounds, votes, and context refs.
- Gateway context: add goal snapshot agent context and an MCP tool for task-level
  drill-in.
- Examples: provide request examples for adversarial branch, mechanism round,
  and persona guidance.
- Docs: document actor/critic usage and shortcut flow choices in goal authoring,
  CLI, distributed runners, and gateway design docs.
- Tests: cover CLI parsing/request generation, TUI grouping, gateway smoke, MCP
  context, and example contract parsing.

## Acceptance Criteria

- Operators can preview or start adversarial actor/critic work from `coat goal
  adversarial` without hand-authoring branch or mechanism JSON.
- The shortcut can target a goal, task, or subgoal and can customize actor count,
  personas, model labels, critic checks, research topics, unifier behavior,
  rounds, and satisfaction threshold.
- Native runner subagents remain disabled by convention; child work is expressed
  as durable COAT task requests.
- TUI and gateway surfaces expose actor/critic context, votes, reviews, and
  unification state.
- Tests fail if the shortcut accepts a single actor or if gateway context stops
  exposing persona/model/runner/session/thread refs.

## Validation

- `cargo test -p coat-cli adversarial`
- `cargo test -p coat-cli`
- `cargo test -p coat-domain examples_parse_against_domain_contracts`
- `npm run --prefix ui/control-plane-web build`
- `npm run --prefix ui/control-plane-web smoke`
- `git diff --check`

## Follow-Ups

None currently. Live Codex, Claude Code, and provider-backed actor execution
remain in the active live-runtime plan.
