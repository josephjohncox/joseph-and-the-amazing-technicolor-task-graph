# 060 Test Review Validator

## Objective

Implement tester, reviewer, validator, and patch-merger workers around evidence, not free-form summaries.

## Implementation

- Tester records commands, exit codes, and artifact paths.
- Reviewer records findings with file and line references.
- Validator applies done criteria and returns `ValidationReport`.
- Patch merger combines only validated artifacts.
- Failed validation becomes retry, child-task request, blocked, or failed according to policy.

## Tests

- Validator enforces artifact existence and minimum score.
- Reviewer findings fail validation when priority is high.
- Patch merger refuses unvalidated artifacts.

## Acceptance

- Validator can run standalone and through Restate.
- Validation reports are schema-valid.
