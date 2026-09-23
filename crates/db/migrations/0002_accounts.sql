-- Per-user accounts, browser sessions, share links, and OAuth for MCP clients.

ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN password_hash TEXT;
ALTER TABLE users ADD COLUMN lichess_id TEXT;
ALTER TABLE users ADD COLUMN lichess_username TEXT;
CREATE UNIQUE INDEX users_email ON users(email) WHERE email IS NOT NULL;
CREATE UNIQUE INDEX users_lichess ON users(lichess_id) WHERE lichess_id IS NOT NULL;

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE INDEX sessions_user ON sessions(user_id);

-- Anyone with the token can view the game and its analysis read-only.
ALTER TABLE games ADD COLUMN share_token TEXT;
CREATE UNIQUE INDEX games_share ON games(share_token) WHERE share_token IS NOT NULL;

-- OAuth 2.1 authorization server for MCP clients (Claude, ChatGPT).
CREATE TABLE oauth_clients (
    client_id     TEXT PRIMARY KEY,
    client_name   TEXT NOT NULL,
    redirect_uris TEXT NOT NULL,          -- JSON array
    metadata_json TEXT NOT NULL,
    created_at    INTEGER NOT NULL
);

CREATE TABLE oauth_codes (
    code_hash      TEXT PRIMARY KEY,
    client_id      TEXT NOT NULL,
    user_id        TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    redirect_uri   TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    resource       TEXT,
    scope          TEXT NOT NULL,
    expires_at     INTEGER NOT NULL
);

CREATE TABLE oauth_tokens (
    token_hash  TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,            -- 'access' | 'refresh'
    client_id   TEXT NOT NULL,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    resource    TEXT,
    scope       TEXT NOT NULL,
    expires_at  INTEGER NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX oauth_tokens_user ON oauth_tokens(user_id);
