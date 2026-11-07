CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE schema_version (
    version    integer     NOT NULL,
    applied_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO schema_version (version) VALUES (1);

CREATE TABLE memories (
    id               uuid PRIMARY KEY,
    memory_type      text        NOT NULL CHECK (memory_type IN ('fact', 'decision', 'problem', 'fix', 'procedure', 'gotcha')),
    title            text        NOT NULL,
    body             text        NOT NULL,
    tags             text[]      NOT NULL DEFAULT '{}',
    project          text        NOT NULL,
    importance       real        NOT NULL DEFAULT 0.5 CHECK (importance >= 0 AND importance <= 1),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    embedding        halfvec(1024),
    embedding_model  text,
    embedding_digest text,
    tsv              tsvector GENERATED ALWAYS AS (
                         to_tsvector('english', title || ' ' || body)
                     ) STORED
);

CREATE INDEX memories_tsv_idx ON memories USING gin (tsv);
CREATE INDEX memories_embedding_idx ON memories USING hnsw (embedding halfvec_cosine_ops);
CREATE INDEX memories_project_idx ON memories (project);

CREATE TABLE links (
    from_id    uuid        NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
    to_id      uuid        NOT NULL REFERENCES memories (id) ON DELETE CASCADE,
    link_type  text        NOT NULL CHECK (link_type IN ('SUPERSEDES', 'SOLVES', 'CAUSES', 'CONTRADICTS')),
    strength   real        NOT NULL DEFAULT 1.0 CHECK (strength >= 0 AND strength <= 1),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (from_id, to_id, link_type)
);

CREATE INDEX links_to_idx ON links (to_id, link_type);
