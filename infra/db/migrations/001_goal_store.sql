CREATE SCHEMA IF NOT EXISTS coat;

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE IF NOT EXISTS coat.goals (
    id uuid PRIMARY KEY,
    title text NOT NULL,
    objective text NOT NULL,
    repo text,
    status text NOT NULL,
    total_tasks integer NOT NULL DEFAULT 0,
    open_tasks integer NOT NULL DEFAULT 0,
    blocked_tasks integer NOT NULL DEFAULT 0,
    failed_tasks integer NOT NULL DEFAULT 0,
    percent_done real NOT NULL DEFAULT 0,
    root_task_id uuid,
    satisfied boolean NOT NULL DEFAULT false,
    satisfaction_score real,
    updated_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    full_state_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    protocol_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    projection_reason text,
    created_at timestamptz NOT NULL DEFAULT now(),
    projected_at timestamptz NOT NULL DEFAULT now(),
    version bigint NOT NULL DEFAULT 0,
    CHECK (status IN (
        'running',
        'waiting_approval',
        'done',
        'blocked',
        'failed',
        'paused',
        'cancelled'
    ))
);

CREATE INDEX IF NOT EXISTS idx_goals_status_projected
    ON coat.goals(status, projected_at DESC);

CREATE INDEX IF NOT EXISTS idx_goals_repo_status
    ON coat.goals(repo, status, projected_at DESC)
    WHERE repo IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_goals_satisfied
    ON coat.goals(satisfied, projected_at DESC);

CREATE INDEX IF NOT EXISTS idx_goals_record_gin
    ON coat.goals USING gin(record_json);

CREATE TABLE IF NOT EXISTS coat.plans (
    id uuid PRIMARY KEY,
    title text NOT NULL,
    objective text NOT NULL,
    repo text,
    status text NOT NULL,
    mode text NOT NULL,
    version integer NOT NULL DEFAULT 1,
    compiled_goal_id uuid,
    updated_at_text text,
    record_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    projected_at timestamptz NOT NULL DEFAULT now(),
    CHECK (status IN (
        'draft',
        'needs_questions',
        'ready_for_review',
        'approved',
        'compiled',
        'superseded',
        'archived'
    )),
    CHECK (mode IN (
        'interactive',
        'autonomous',
        'human_steered',
        'research_first',
        'implementation_ready'
    ))
);

CREATE INDEX IF NOT EXISTS idx_plans_status_projected
    ON coat.plans(status, projected_at DESC);

CREATE INDEX IF NOT EXISTS idx_plans_repo_status
    ON coat.plans(repo, status, projected_at DESC)
    WHERE repo IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_plans_compiled_goal
    ON coat.plans(compiled_goal_id)
    WHERE compiled_goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_plans_record_gin
    ON coat.plans USING gin(record_json);

CREATE TABLE IF NOT EXISTS coat.tasks (
    id uuid PRIMARY KEY,
    goal_id uuid NOT NULL REFERENCES coat.goals(id) ON DELETE CASCADE,
    parent_task_id uuid,
    subgoal_id text,
    title text NOT NULL,
    role text NOT NULL,
    status text NOT NULL,
    purpose_kind text NOT NULL,
    depth integer NOT NULL DEFAULT 0,
    priority text NOT NULL,
    priority_rank smallint NOT NULL DEFAULT 3,
    attempts integer NOT NULL DEFAULT 0,
    runnable boolean NOT NULL DEFAULT false,
    tags text[] NOT NULL DEFAULT ARRAY[]::text[],
    result_uri text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK (status IN (
        'pending',
        'runnable',
        'running',
        'needs_validation',
        'waiting_approval',
        'done',
        'blocked',
        'failed',
        'cancelled'
    )),
    CHECK (purpose_kind IN (
        'work',
        'review',
        'unification',
        'actor_retry',
        'candidate_branch',
        'branch_vote',
        'branch_unification',
        'research'
    ))
);

CREATE INDEX IF NOT EXISTS idx_tasks_goal_status
    ON coat.tasks(goal_id, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_tasks_parent
    ON coat.tasks(parent_task_id)
    WHERE parent_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_role_status
    ON coat.tasks(role, status, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_tasks_subgoal
    ON coat.tasks(goal_id, subgoal_id)
    WHERE subgoal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_tasks_runnable_priority
    ON coat.tasks(goal_id, runnable, priority_rank DESC, depth ASC);

CREATE INDEX IF NOT EXISTS idx_tasks_record_gin
    ON coat.tasks USING gin(record_json);

CREATE TABLE IF NOT EXISTS coat.goal_events (
    id uuid PRIMARY KEY,
    goal_id uuid NOT NULL REFERENCES coat.goals(id) ON DELETE CASCADE,
    task_id uuid,
    sequence bigint NOT NULL,
    kind text NOT NULL,
    message text NOT NULL,
    actor text,
    idempotency_key text NOT NULL,
    created_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_events_goal_sequence
    ON coat.goal_events(goal_id, sequence);

CREATE INDEX IF NOT EXISTS idx_goal_events_goal_idempotency
    ON coat.goal_events(goal_id, idempotency_key);

CREATE INDEX IF NOT EXISTS idx_goal_events_kind_recorded
    ON coat.goal_events(kind, recorded_at DESC);

CREATE INDEX IF NOT EXISTS idx_goal_events_task
    ON coat.goal_events(task_id)
    WHERE task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS coat.approvals (
    id uuid PRIMARY KEY,
    goal_id uuid NOT NULL REFERENCES coat.goals(id) ON DELETE CASCADE,
    task_id uuid,
    status text NOT NULL,
    risk text NOT NULL,
    reason text NOT NULL,
    requested_action text NOT NULL,
    updated_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (status IN ('pending', 'approved', 'rejected')),
    CHECK (risk IN ('low', 'medium', 'high', 'critical'))
);

CREATE INDEX IF NOT EXISTS idx_approvals_goal_status
    ON coat.approvals(goal_id, status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_approvals_task
    ON coat.approvals(task_id)
    WHERE task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS coat.artifacts (
    id uuid PRIMARY KEY,
    goal_id uuid NOT NULL REFERENCES coat.goals(id) ON DELETE CASCADE,
    task_id uuid,
    artifact_type text NOT NULL,
    uri text NOT NULL,
    description text NOT NULL,
    git_remote text,
    git_ref text,
    git_commit_sha text,
    object_bucket text,
    object_key text,
    sha256 text,
    created_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_artifacts_goal_type
    ON coat.artifacts(goal_id, artifact_type, recorded_at DESC);

CREATE INDEX IF NOT EXISTS idx_artifacts_task
    ON coat.artifacts(task_id)
    WHERE task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_artifacts_git_ref
    ON coat.artifacts(git_ref)
    WHERE git_ref IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_artifacts_object
    ON coat.artifacts(object_bucket, object_key)
    WHERE object_bucket IS NOT NULL AND object_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS coat.goal_outbox (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    goal_id uuid NOT NULL REFERENCES coat.goals(id) ON DELETE CASCADE,
    task_id uuid,
    topic text NOT NULL,
    dedupe_key text,
    payload jsonb NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    attempts integer NOT NULL DEFAULT 0,
    available_at timestamptz NOT NULL DEFAULT now(),
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz,
    CHECK (status IN ('pending', 'sending', 'sent', 'failed', 'dead_letter'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_goal_outbox_dedupe
    ON coat.goal_outbox(topic, dedupe_key)
    WHERE dedupe_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_goal_outbox_ready
    ON coat.goal_outbox(status, available_at)
    WHERE status IN ('pending', 'failed');
