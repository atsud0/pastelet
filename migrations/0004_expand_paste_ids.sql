CREATE TABLE pastes_new (
    id TEXT PRIMARY KEY NOT NULL
        CHECK (length(id) IN (8, 22) AND id NOT GLOB '*[^A-Za-z0-9]*'),
    title TEXT CHECK (title IS NULL OR length(title) <= 200),
    content TEXT NOT NULL CHECK (length(CAST(content AS BLOB)) > 0),
    language TEXT NOT NULL CHECK (length(language) > 0),
    password_hash TEXT,
    burn_after_read INTEGER NOT NULL DEFAULT 0 CHECK (burn_after_read IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    expires_at INTEGER CHECK (expires_at IS NULL OR expires_at > created_at)
) STRICT;

INSERT INTO pastes_new SELECT id, title, content, language, password_hash,
    burn_after_read, created_at, expires_at FROM pastes;

CREATE TABLE paste_tokens_new (
    token_hash BLOB PRIMARY KEY NOT NULL CHECK (length(token_hash) = 32),
    paste_id TEXT NOT NULL REFERENCES pastes_new(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('access', 'confirm')),
    expires_at INTEGER NOT NULL
) STRICT;

INSERT INTO paste_tokens_new SELECT token_hash, paste_id, kind, expires_at FROM paste_tokens;

CREATE TABLE pdf_files_new (
    paste_id TEXT PRIMARY KEY NOT NULL REFERENCES pastes_new(id) ON DELETE CASCADE,
    storage_name TEXT UNIQUE NOT NULL,
    data BLOB NOT NULL CHECK (length(data) >= 5 AND substr(data, 1, 5) = X'255044462D')
) STRICT;

INSERT INTO pdf_files_new SELECT paste_id, storage_name, data FROM pdf_files;

DROP TABLE pdf_files;
DROP TABLE paste_tokens;
DROP TABLE pastes;
ALTER TABLE pastes_new RENAME TO pastes;
ALTER TABLE paste_tokens_new RENAME TO paste_tokens;
ALTER TABLE pdf_files_new RENAME TO pdf_files;

CREATE INDEX idx_pastes_expires_at ON pastes (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX paste_tokens_paste ON paste_tokens(paste_id);
CREATE INDEX paste_tokens_expiry ON paste_tokens(expires_at);
