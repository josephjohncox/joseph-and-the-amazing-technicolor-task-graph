# 060 Test Review Validator

## Objective

Implement tester, reviewer, validator, and patch-merger workers around evidence, not free-form summaries.

## Implementation

- Tester records commands, exit codes, and artifact paths.
- Reviewer records findings with file and line references.
- Reviewer and unifier workers return `ReviewOutput` with decision, reward, findings, and retry recommendation.
- Add doctrine coverage fixtures for formal-methods, type-soundness, hypothesis-testing, DDD, readability, abstraction, and security review objectives.
- Add behavioral testing doctrine that reviewer/tester agents can inject as `behavioral_testing`; it must check the end-to-end objective, user/operator workflow, state transitions, and failure modes rather than only UI/API/file existence.
- Validator applies done criteria and returns `ValidationReport`.
- Validation treats review task completion separately from review acceptance; `changes_requested`, `blocked`, or `inconclusive` decisions keep goal satisfaction false.
- Validation requires passing `test_evidence` for work-like and tester tasks when `done_criteria.tests_pass=true`.
- Validation blocks high/critical or priority 0/1 review findings even if the reviewer decision says `accept`.
- Patch merger and branch-unifier votes can select only declared branch candidates that have already validated successfully.
- Local domain tests model review-unifier and patch-merger checkpoint branch evidence with typed git refs, without requiring live git worktrees.
- Failed validation becomes retry, child-task request, blocked, or failed according to policy.

## Tests

- Validator enforces artifact existence and minimum score.
- Reviewer findings fail validation when priority is high.
- Standard `behavioral_testing` steering creates a tester task and validation fails when the review omits `testing.behavioral_end_to_end` or `gate.behavioral_coverage`.
- Doctrine fixtures must cover every strict objective and gate, and must fail for meaningful behavioral gaps rather than passing on presence-only evidence.
- Critic decisions block `SatisfactionReport.satisfied` even when reward is high.
- Patch merger refuses unknown or unvalidated branch candidates.
- Review unification and patch-merger branch selection preserve typed git checkpoint branch refs in state and goal-store projections.

## Follow-Ups

None currently. Reviewer fixture growth and live git-worktree coverage are
tracked through the active master plan as live worker and provisioning evidence
appears.

## Acceptance

- Validator can run standalone and through Restate.
- Validation reports are schema-valid.
