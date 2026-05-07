# 150 Durable Planning Mode

## Objective

Add durable planning-mode artifacts so operators can draft, revise, review, and compile plans before submitting executable goals.

## Implementation

- Add domain contracts for `DurablePlan`, `PlanRevision`, `PlanQuestion`, `PlanDecision`, plan status, planning mode, draft/revision/compile requests, summaries, and responses.
- Add JSON schema generation for plan contracts.
- Persist plans in `coat-goal-store` with JSONL replay and Postgres projection support.
- Add Postgres migration table and indexes for durable plans.
- Add `coat plan draft/list/show/revise/compile` CLI commands.
- Add SPA plan views and MCP tools for plan listing, inspection, and compilation.
- Document the plan-to-goal workflow.
- Add `source_plan_id` so operators can branch planning history and compile branch candidates into distinct goals.
- Add SPA branch comparison and selection rows for plans with `source_plan_id`.

## Tests

- Unit test durable plan revision and compilation into `GoalSpec`.
- Unit test goal-store plan indexing.
- Run schema generation, Rust tests, TypeScript compile, protobuf lint, and whitespace checks.

## Follow-Ups

- Add backend-backed branch voting and winner-selection workflows for compiled plan candidates.

## Acceptance

- A rough planning prompt can become a durable plan record.
- The plan can be revised without submitting a goal.
- The plan can compile into a valid `GoalSpec`.
- Branched plans preserve `source_plan_id` and can compile to a distinct `goal_id`.
- The SPA and MCP can inspect plans without owning state.
- Restate remains authoritative only after a compiled goal is submitted.
