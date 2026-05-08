# 150 Durable Planning Mode

## Objective

Add durable planning-mode artifacts so operators can draft, revise, review, and compile plans before submitting executable goals.

## Implementation

- Add domain contracts for `DurablePlan`, `PlanRevision`, `PlanQuestion`, `PlanDecision`, plan status, planning mode, draft/revision/compile requests, summaries, and responses.
- Add JSON schema generation for plan contracts.
- Persist plans in `coat-goal-store` with JSONL replay and Postgres projection support.
- Add Postgres migration table and indexes for durable plans.
- Add `coat plan draft/list/show/revise/compile/vote-candidate/select-candidate` CLI commands.
- Add SPA plan views and MCP tools for plan listing, inspection, and compilation.
- Document the plan-to-goal workflow.
- Add `source_plan_id` so operators can branch planning history and compile branch candidates into distinct goals.
- Add SPA branch comparison and selection rows for plans with `source_plan_id`.
- Add backend plan-candidate vote and selection contracts, goal-store endpoints, CLI commands, schemas, and examples.

## Tests

- Unit test durable plan revision and compilation into `GoalSpec`.
- Unit test durable plan candidate voting and selection.
- Unit test goal-store plan indexing.
- Run schema generation, Rust tests, TypeScript compile, protobuf lint, and whitespace checks.

## Evidence

- `cargo test -p coat-domain durable_plan_records_candidate_votes_and_selection -- --nocapture`
- `cargo test -p coat-domain examples_parse_against_domain_contracts -- --nocapture`
- `cargo test -p coat-goal-store plan_candidate_handlers_enforce_branch_and_compilation -- --nocapture`
- `cargo test --workspace`
- `cargo check --workspace`
- `sh scripts/coat-doc-gardener.sh`
- `git diff --check`
- `buf lint` was not run because `buf` is not installed in this local environment.

## Follow-Ups

- None currently.

## Acceptance

- A rough planning prompt can become a durable plan record.
- The plan can be revised without submitting a goal.
- The plan can compile into a valid `GoalSpec`.
- Branched plans preserve `source_plan_id` and can compile to a distinct `goal_id`.
- A source plan can record reviewer votes for compiled branch candidates and select a winning candidate through `coat plan`.
- The SPA and MCP can inspect plans without owning state.
- Restate remains authoritative only after a compiled goal is submitted.
