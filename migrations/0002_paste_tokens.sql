CREATE TABLE paste_tokens (
    token_hash BLOB PRIMARY KEY NOT NULL CHECK (length(token_hash) = 32),
    paste_id TEXT NOT NULL REFERENCES pastes(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('access', 'confirm')),
    expires_at INTEGER NOT NULL
) STRICT;

CREATE INDEX paste_tokens_paste ON paste_tokens(paste_id);
CREATE INDEX paste_tokens_expiry ON paste_tokens(expires_at);
