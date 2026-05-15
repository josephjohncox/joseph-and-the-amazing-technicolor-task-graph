CREATE SCHEMA IF NOT EXISTS coat;

CREATE TABLE IF NOT EXISTS coat.control_chat_turns (
    id uuid PRIMARY KEY,
    idempotency_key text,
    draft_id uuid,
    session_id text NOT NULL,
    goal_id uuid,
    mode text NOT NULL DEFAULT 'general',
    role text NOT NULL,
    content text NOT NULL,
    provider text,
    model text,
    created_at_text text,
    payload_json jsonb NOT NULL DEFAULT '{}'::jsonb,
    record_json jsonb NOT NULL,
    recorded_at timestamptz NOT NULL DEFAULT now(),
    CHECK (role IN ('user', 'assistant'))
);

ALTER TABLE coat.control_chat_turns
    ADD COLUMN IF NOT EXISTS idempotency_key text;

ALTER TABLE coat.control_chat_turns
    ADD COLUMN IF NOT EXISTS draft_id uuid;

CREATE UNIQUE INDEX IF NOT EXISTS idx_control_chat_turns_idempotency
    ON coat.control_chat_turns(idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_control_chat_turns_session
    ON coat.control_chat_turns(session_id, recorded_at ASC, id ASC);

CREATE INDEX IF NOT EXISTS idx_control_chat_turns_goal
    ON coat.control_chat_turns(goal_id, recorded_at DESC)
    WHERE goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_control_chat_turns_draft
    ON coat.control_chat_turns(draft_id, recorded_at ASC)
    WHERE draft_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_control_chat_turns_payload_gin
    ON coat.control_chat_turns USING gin(payload_json);
