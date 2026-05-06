CREATE SCHEMA IF NOT EXISTS coat;

CREATE TABLE IF NOT EXISTS coat.event_sources (
    id text PRIMARY KEY,
    source_key text NOT NULL UNIQUE,
    kind text NOT NULL,
    display_name text NOT NULL,
    status text NOT NULL DEFAULT 'active',
    auth_policy jsonb NOT NULL DEFAULT '{}'::jsonb,
    route_policy jsonb NOT NULL DEFAULT '{}'::jsonb,
    schedule jsonb,
    cursor_state jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    disabled_at timestamptz,
    CHECK (status IN ('active', 'paused', 'disabled'))
);

CREATE INDEX IF NOT EXISTS idx_event_sources_kind_status
    ON coat.event_sources(kind, status);

CREATE INDEX IF NOT EXISTS idx_event_sources_route_gin
    ON coat.event_sources USING gin(route_policy);

CREATE TABLE IF NOT EXISTS coat.external_events (
    id text PRIMARY KEY,
    source_id text NOT NULL,
    source_key text NOT NULL,
    event_type text NOT NULL,
    subject text,
    dedupe_key text,
    cloud_event_id text,
    cloud_event_source text,
    observed_at timestamptz NOT NULL DEFAULT now(),
    occurred_at timestamptz,
    payload jsonb NOT NULL,
    headers jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_external_events_source_dedupe
    ON coat.external_events(source_key, dedupe_key)
    WHERE dedupe_key IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_external_events_cloudevent
    ON coat.external_events(cloud_event_source, cloud_event_id)
    WHERE cloud_event_source IS NOT NULL AND cloud_event_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_external_events_type_observed
    ON coat.external_events(event_type, observed_at DESC);

CREATE TABLE IF NOT EXISTS coat.triggered_goals (
    id uuid PRIMARY KEY,
    external_event_id text REFERENCES coat.external_events(id) ON DELETE SET NULL,
    route_mode text NOT NULL,
    status text NOT NULL,
    goal_id uuid,
    target_goal_id uuid,
    template jsonb NOT NULL DEFAULT '{}'::jsonb,
    result jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    CHECK (route_mode IN (
        'record_only',
        'create_goal',
        'create_research_goal',
        'steer_goal',
        'human_review'
    )),
    CHECK (status IN ('recorded', 'submitted', 'awaiting_human_review', 'deduped', 'failed'))
);

CREATE INDEX IF NOT EXISTS idx_triggered_goals_goal
    ON coat.triggered_goals(goal_id, created_at DESC)
    WHERE goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_triggered_goals_target_goal
    ON coat.triggered_goals(target_goal_id, created_at DESC)
    WHERE target_goal_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS coat.event_outbox (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    external_event_id text REFERENCES coat.external_events(id) ON DELETE SET NULL,
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

CREATE UNIQUE INDEX IF NOT EXISTS idx_event_outbox_dedupe
    ON coat.event_outbox(topic, dedupe_key)
    WHERE dedupe_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_event_outbox_ready
    ON coat.event_outbox(status, available_at)
    WHERE status IN ('pending', 'failed');
