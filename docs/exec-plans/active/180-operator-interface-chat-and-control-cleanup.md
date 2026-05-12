# 180 Operator Interface Chat And Control Cleanup

## Objective

Make the COAT operator interfaces understandable and usable for real work. The
SPA and TUI should make the selected goal, active draft, current chat scope,
task graph state, runner state, blocked work, next action, and evidence obvious
without requiring operators to understand raw JSON, Restate internals, or hidden
session mechanics.

## Product Questions

- What is the current operator context: workspace chat, selected goal, selected
  draft, selected task, or selected subgoal?
- What happens when the operator presses Enter in the TUI, and which panel has
  focus?
- While chat is generating, did the prompt send, can the operator keep typing,
  and where is the pending response visible?
- Which draft is active, which chat response produced it, and what exact payload
  will be submitted?
- After submitting a draft, why is the selected goal not immediately visible in
  the goal list, graph, and subgoal views?
- How can an operator inspect, edit, discard, or submit goal and subgoal drafts
  without opening raw JSON first?
- Which controls are primary actions for most operators, and which controls are
  advanced compiler controls?
- What does the runner fleet look like right now: registered runners, capacity,
  endpoints, capabilities, and current task pressure?
- What wording is leaking from implementation or prompt language into the UI?

## Decisions

- The TUI uses explicit focus. Enter sends chat only when the chat input is
  focused; navigation keys should not accidentally submit prompts.
- Sent chat text clears from the input immediately, appears in the chat log, and
  keeps the chat pinned to the newest turn unless the operator has scrolled away.
- Busy chat is visible as an activity/status indicator, not by leaving stale
  input in the composer.
- The SPA Chat panel owns draft review. Drafts are first-class pending objects
  with review, submit, and discard actions.
- Submitting a draft selects the returned goal, refreshes goals and snapshot
  queries, and keeps a submitted-draft fallback visible until goal-store projects
  the durable state.
- The default control surface presents primary actions first: resume/action,
  steer/evaluate, submit draft, refresh, and inspect evidence. Restart, branch,
  mechanism rounds, raw payload inspection, and destructive actions are advanced.
- User-facing copy says `runner`, `model route`, `task`, `subgoal`, or
  `workstream` as appropriate. Avoid `lane` in product surfaces and guidance
  unless naming an internal field such as `lane_policies` or `labels.lane`.

## Implementation Workstreams

- TUI: fix chat input clearing, busy state, focus/navigation, selected goal
  context, draft review, and tests.
- SPA chat: fix durable chat session loading, active draft state, submit/discard
  flow, chat scroll, and goal refresh after submit.
- SPA graph/control: simplify goal, subgoal, task graph, runner, and control
  surfaces around current state, next action, blockers, evidence, and advanced
  controls.
- Terminology: update AGENTS/docs/UI wording and add a guard where practical.
- Behavioral tests: cover operator outcomes across TUI units, SPA smoke, and
  Playwright flows.

## Acceptance Criteria

- TUI Enter behavior is deterministic and covered by unit tests.
- TUI input clears immediately after send, while sent text remains visible in
  chat history during generation.
- SPA chat history survives goal selection changes and draft submission.
- SPA clearly shows workspace chat vs selected-goal chat vs active draft.
- A chat-created draft can be reviewed, submitted, and discarded without raw JSON
  as the primary path.
- Submitted goals immediately become the selected goal and appear in goal/graph
  context even before projection catches up.
- Goal, subgoal, task, runner, graph, blocker, evidence, and next-action state
  are visible from the main operator surfaces.
- Flow controls are separated into primary actions and advanced controls.
- User-facing copy no longer uses ambiguous lane wording.

## Validation

- `cargo test -p coat-cli tui`
- `npm run --prefix ui/control-plane-web build`
- `npm run --prefix ui/control-plane-web smoke`
- `npm run --prefix ui/control-plane-web test:e2e`
- `make scenario-e2e-ui`
- `make docs-check`
- `git diff --check`

## Follow-Ups

- Completed: add a richer TUI selected-goal outline for projected subgoals,
  tasks, and compute graph nodes so terminal operators can navigate more than a
  flat latest-goals list.
- Completed: add first-class SPA draft editing fields for title, objective,
  evidence requirements, and constraints before submit.
- Add a live backend E2E proving goal-store refresh with real Compose services
  after deterministic fixture coverage is stable.
