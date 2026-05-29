# Live Bootstrap Examples

These fixtures are small, fixed-id `GoalSpec` and thunk payloads used by
`scripts/coat-bootstrap-live-scenarios.sh`.

The script has two paths:

- live coordinator goals through `coat goal submit` and `coat goal thunk create`;
- deterministic scenario fixture projections through `coat scenario seed`.

Use the live path to prove the local stack accepts coordinator-owned work. Use
the fixture path to populate the SPA/TUI with navigable examples for completed
work, pending actions, approvals, thunk resume history, fanout, fork/join,
signal-driven work, and recovery states.

Run:

```sh
coat deploy local up --allow-stub-runners
sh scripts/coat-bootstrap-live-scenarios.sh
```

The fixed live coordinator goal ids are:

- `00000000-0000-4000-8000-000000004004`: completed executor lifecycle.
- `00000000-0000-4000-8000-000000004002`: approval pending task.
- `00000000-0000-4000-8000-000000004003`: human prompt pending thunk.

The script is idempotent by default. It skips fixed live coordinator goals when
goal-store already has them, and scenario fixture seeding uses stable
idempotency keys.
