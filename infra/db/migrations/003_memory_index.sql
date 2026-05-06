CREATE SCHEMA IF NOT EXISTS coat;

CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS coat.memory_documents (
    id uuid PRIMARY KEY,
    goal_id uuid,
    task_id uuid,
    source_uri text NOT NULL,
    memory_kind text NOT NULL,
    title text,
    content text NOT NULL,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    embedding_model text,
    embedding halfvec(3072),
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_memory_documents_goal
    ON coat.memory_documents(goal_id, created_at DESC)
    WHERE goal_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memory_documents_task
    ON coat.memory_documents(task_id, created_at DESC)
    WHERE task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_memory_documents_metadata
    ON coat.memory_documents USING gin(metadata);

CREATE INDEX IF NOT EXISTS idx_memory_documents_embedding_cosine
    ON coat.memory_documents USING hnsw (embedding halfvec_cosine_ops)
    WHERE embedding IS NOT NULL;

CREATE TABLE IF NOT EXISTS coat.memory_edges (
    id uuid PRIMARY KEY,
    from_memory_id uuid NOT NULL REFERENCES coat.memory_documents(id) ON DELETE CASCADE,
    to_memory_id uuid NOT NULL REFERENCES coat.memory_documents(id) ON DELETE CASCADE,
    relation text NOT NULL,
    confidence double precision NOT NULL DEFAULT 1.0,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (from_memory_id, to_memory_id, relation)
);

CREATE INDEX IF NOT EXISTS idx_memory_edges_to
    ON coat.memory_edges(to_memory_id);
