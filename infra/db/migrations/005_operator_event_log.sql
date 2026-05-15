CREATE TABLE IF NOT EXISTS coat.operator_events (
    id uuid PRIMARY KEY,
    event_type text NOT NULL,
    actor_kind text NOT NULL,
    actor_id text NOT NULL,
    goal_id uuid,
    task_id uuid,
    transition text NOT NULL,
    idempotency_key text NOT NULL,
    causation_id text,
    correlation_id text,
    restate_invocation_id text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at_text text NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

ALTER TABLE coat.operator_events
    ADD COLUMN IF NOT EXISTS record_json jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE UNIQUE INDEX IF NOT EXISTS idx_operator_events_idempotency
    ON coat.operator_events(idempotency_key);

CREATE INDEX IF NOT EXISTS idx_operator_events_goal_recorded
    ON coat.operator_events(goal_id, recorded_at DESC)
    WHERE goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_operator_events_actor
    ON coat.operator_events(actor_kind, actor_id, recorded_at DESC);

CREATE INDEX IF NOT EXISTS idx_operator_events_type_recorded
    ON coat.operator_events(event_type, recorded_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_events_goal_idempotency_unique
    ON coat.goal_events(goal_id, idempotency_key);

ALTER TABLE coat.tasks
    DROP CONSTRAINT IF EXISTS tasks_status_check;

ALTER TABLE coat.tasks
    ADD CONSTRAINT tasks_status_check CHECK (status IN (
        'pending',
        'runnable',
        'running',
        'needs_validation',
        'waiting_approval',
        'waiting_input',
        'done',
        'blocked',
        'failed',
        'cancelled'
    ));

ALTER TABLE coat.approvals
    DROP CONSTRAINT IF EXISTS approvals_status_check;

ALTER TABLE coat.approvals
    ADD CONSTRAINT approvals_status_check CHECK (status IN (
        'pending',
        'approved',
        'rejected',
        'cancelled'
    ));
