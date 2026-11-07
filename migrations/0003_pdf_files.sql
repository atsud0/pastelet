CREATE TABLE pdf_files (
    paste_id TEXT PRIMARY KEY NOT NULL REFERENCES pastes(id) ON DELETE CASCADE,
    storage_name TEXT UNIQUE NOT NULL,
    data BLOB NOT NULL CHECK (length(data) >= 5 AND substr(data, 1, 5) = X'255044462D')
) STRICT;
