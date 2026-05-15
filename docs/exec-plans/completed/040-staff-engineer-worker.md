# 040 Staff Engineer Worker

## Objective

Use `@ctxr/agent-staff-engineer` as a specialized issue-to-PR worker after dependency verification.

## Implementation

- Verify `@ctxr/kit` and `@ctxr/agent-staff-engineer` availability.
- Install the bundle only into isolated target repos.
- Model requests as `goal_id`, `task_id`, `repo_path`, `issue_ref`, `instruction`, and `max_minutes`.
- Return `status`, `branch`, `pr_url`, `summary`, and `unresolved_blockers`.
- Preserve human gates for merge, tracker Done, and dangerous operations.

## Tests

- `/verify` reports package availability without mutating the repo.
- Stub `/run-task` returns a blocked result with actionable setup blockers.
- Live install tests are gated by credentials and explicit environment.

## Follow-Ups

- Superseded by the active master plan:
  `docs/exec-plans/active/160-live-durable-runtime-and-execution.md`.
  Codex App Server remains the first live worker reference; staff-engineer live
  package verification and issue-to-PR smoke remain gated until package behavior,
  isolated repo install, tracker auth, and Claude Code auth distribution are
  approved.

## Acceptance

- Worker can be deployed in stub mode.
- Coordinator can route staff-engineer tasks without knowing Claude Code details.
