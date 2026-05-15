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

CREATE TABLE IF NOT EXISTS coat.operator_action_queue (
    action_id text PRIMARY KEY,
    kind text NOT NULL,
    goal_id uuid NOT NULL,
    task_id uuid,
    title text NOT NULL,
    question text NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    allowed_resolutions text[] NOT NULL DEFAULT ARRAY[]::text[],
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    projected_at timestamptz NOT NULL DEFAULT now(),
    CHECK (status IN ('pending', 'open', 'resolved', 'completed', 'cancelled')),
    CHECK (kind IN (
        'accept_draft',
        'resume_thunk',
        'resolve_approval',
        'restart_task',
        'replan_task',
        'select_branch',
        'cancel_goal'
    ))
);

CREATE INDEX IF NOT EXISTS idx_operator_action_queue_goal_status
    ON coat.operator_action_queue(goal_id, status, projected_at DESC);

CREATE INDEX IF NOT EXISTS idx_operator_action_queue_task
    ON coat.operator_action_queue(task_id)
    WHERE task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_operator_action_queue_kind_status
    ON coat.operator_action_queue(kind, status, projected_at DESC);

CREATE TABLE IF NOT EXISTS coat.goal_drafts (
    id uuid PRIMARY KEY,
    idempotency_key text,
    session_id text,
    plan_id uuid,
    goal_id uuid,
    title text NOT NULL,
    objective text NOT NULL,
    status text NOT NULL DEFAULT 'draft',
    created_at_text text,
    updated_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    projected_at timestamptz NOT NULL DEFAULT now(),
    CHECK (status IN (
        'draft',
        'needs_questions',
        'ready_for_review',
        'approved',
        'compiled',
        'accepted',
        'discarded',
        'superseded',
        'archived'
    ))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_drafts_idempotency
    ON coat.goal_drafts(idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_goal_drafts_session
    ON coat.goal_drafts(session_id, projected_at DESC)
    WHERE session_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_goal_drafts_goal
    ON coat.goal_drafts(goal_id, projected_at DESC)
    WHERE goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_goal_drafts_plan
    ON coat.goal_drafts(plan_id, projected_at DESC)
    WHERE plan_id IS NOT NULL;

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
