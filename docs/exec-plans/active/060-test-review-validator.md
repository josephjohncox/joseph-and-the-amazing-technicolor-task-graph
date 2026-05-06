# 060 Test Review Validator

## Objective

Implement tester, reviewer, validator, and patch-merger workers around evidence, not free-form summaries.

## Implementation

- Tester records commands, exit codes, and artifact paths.
- Reviewer records findings with file and line references.
- Reviewer and unifier workers return `ReviewOutput` with decision, reward, findings, and retry recommendation.
- Validator applies done criteria and returns `ValidationReport`.
- Validation treats review task completion separately from review acceptance; `changes_requested`, `blocked`, or `inconclusive` decisions keep goal satisfaction false.
- Patch merger combines only validated artifacts.
- Failed validation becomes retry, child-task request, blocked, or failed according to policy.

## Tests

- Validator enforces artifact existence and minimum score.
- Reviewer findings fail validation when priority is high.
- Critic decisions block `SatisfactionReport.satisfied` even when reward is high.
- Patch merger refuses unvalidated artifacts.

## Acceptance

- Validator can run standalone and through Restate.
- Validation reports are schema-valid.
