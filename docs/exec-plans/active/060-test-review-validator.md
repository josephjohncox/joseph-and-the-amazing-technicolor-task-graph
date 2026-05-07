# 060 Test Review Validator

## Objective

Implement tester, reviewer, validator, and patch-merger workers around evidence, not free-form summaries.

## Implementation

- Tester records commands, exit codes, and artifact paths.
- Reviewer records findings with file and line references.
- Reviewer and unifier workers return `ReviewOutput` with decision, reward, findings, and retry recommendation.
- Validator applies done criteria and returns `ValidationReport`.
- Validation treats review task completion separately from review acceptance; `changes_requested`, `blocked`, or `inconclusive` decisions keep goal satisfaction false.
- Validation requires passing `test_evidence` for work-like and tester tasks when `done_criteria.tests_pass=true`.
- Validation blocks high/critical or priority 0/1 review findings even if the reviewer decision says `accept`.
- Patch merger and branch-unifier votes can select only declared branch candidates that have already validated successfully.
- Failed validation becomes retry, child-task request, blocked, or failed according to policy.

## Tests

- Validator enforces artifact existence and minimum score.
- Reviewer findings fail validation when priority is high.
- Critic decisions block `SatisfactionReport.satisfied` even when reward is high.
- Patch merger refuses unknown or unvalidated branch candidates.

## Follow-Ups

- Add richer reviewer fixtures for formal-methods, type-soundness, hypothesis-testing, DDD, readability, abstraction, and security doctrines.
- Add patch-merger and review-unifier tests over real git checkpoint branches once live git worktrees are part of CI.

## Acceptance

- Validator can run standalone and through Restate.
- Validation reports are schema-valid.
