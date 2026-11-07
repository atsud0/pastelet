CREATE TABLE pastes (
    id TEXT PRIMARY KEY NOT NULL
        CHECK (length(id) = 8 AND id NOT GLOB '*[^A-Za-z0-9]*'),
    title TEXT CHECK (title IS NULL OR length(title) <= 200),
    content TEXT NOT NULL CHECK (length(CAST(content AS BLOB)) > 0),
    language TEXT NOT NULL CHECK (length(language) > 0),
    password_hash TEXT,
    burn_after_read INTEGER NOT NULL DEFAULT 0 CHECK (burn_after_read IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER CHECK (expires_at IS NULL OR expires_at > created_at)
) STRICT;

CREATE INDEX idx_pastes_expires_at ON pastes (expires_at)
    WHERE expires_at IS NOT NULL;
