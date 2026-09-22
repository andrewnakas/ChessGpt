-- chessgpt schema v1. Portable SQL: TEXT ids (UUIDv7), INTEGER ms timestamps, JSON in TEXT.

CREATE TABLE users (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,              -- 'local' | 'account'
    display_name TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);

CREATE TABLE user_settings (
    user_id           TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    elo               INTEGER NOT NULL DEFAULT 1500,
    lichess_username  TEXT,
    chesscom_username TEXT,
    explorer_enabled  INTEGER NOT NULL DEFAULT 1,
    lichess_token_enc TEXT,                  -- personal API token, encrypted
    updated_at        INTEGER NOT NULL
);

CREATE TABLE providers (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,
    label       TEXT NOT NULL,
    base_url    TEXT NOT NULL,
    model       TEXT NOT NULL,
    api_key_enc TEXT,
    is_default  INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);

CREATE TABLE games (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    source       TEXT NOT NULL,
    source_id    TEXT,
    pgn          TEXT NOT NULL,
    start_fen    TEXT NOT NULL,
    white        TEXT NOT NULL,
    black        TEXT NOT NULL,
    white_elo    INTEGER,
    black_elo    INTEGER,
    result       TEXT NOT NULL,
    date         TEXT,
    time_control TEXT,
    eco          TEXT,
    opening_name TEXT,
    user_side    TEXT,
    ply_count    INTEGER NOT NULL,
    imported_at  INTEGER NOT NULL,
    UNIQUE (user_id, source, source_id)
);
CREATE INDEX games_user_imported ON games(user_id, imported_at DESC);

-- One row per distinct position (FEN without move counters).
CREATE TABLE positions (
    id          INTEGER PRIMARY KEY,
    fen_key     TEXT NOT NULL UNIQUE,
    phase       TEXT NOT NULL,
    piece_count INTEGER NOT NULL
);

CREATE TABLE game_moves (
    game_id            TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    ply                INTEGER NOT NULL,
    san                TEXT NOT NULL,
    uci                TEXT NOT NULL,
    position_before_id INTEGER NOT NULL REFERENCES positions(id),
    position_after_id  INTEGER NOT NULL REFERENCES positions(id),
    clock_ms           INTEGER,
    PRIMARY KEY (game_id, ply)
);
CREATE INDEX game_moves_before ON game_moves(position_before_id);

-- Engine cache: the deepest search per (position, engine, multipv).
CREATE TABLE analyses (
    position_id INTEGER NOT NULL REFERENCES positions(id),
    engine      TEXT NOT NULL,
    multipv     INTEGER NOT NULL,
    depth       INTEGER NOT NULL,
    nodes       INTEGER NOT NULL,
    time_ms     INTEGER NOT NULL,
    best_move   TEXT,
    lines_json  TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (position_id, engine, multipv)
);

CREATE TABLE game_analyses (
    id               TEXT PRIMARY KEY,
    game_id          TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    status           TEXT NOT NULL,
    elo              INTEGER NOT NULL,
    tier             TEXT NOT NULL,
    user_side        TEXT,
    engine           TEXT NOT NULL,
    start_score_json TEXT,
    white_accuracy   REAL,
    black_accuracy   REAL,
    key_moments_json TEXT NOT NULL DEFAULT '[]',
    review_json      TEXT,
    error            TEXT,
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
CREATE INDEX game_analyses_game ON game_analyses(game_id, created_at DESC);

CREATE TABLE move_evals (
    analysis_id       TEXT NOT NULL REFERENCES game_analyses(id) ON DELETE CASCADE,
    ply               INTEGER NOT NULL,
    classification    TEXT NOT NULL,
    lichess_judgement TEXT,
    delta_wc          REAL NOT NULL,
    phase             TEXT NOT NULL,
    is_key_moment     INTEGER NOT NULL DEFAULT 0,
    data_json         TEXT NOT NULL,
    PRIMARY KEY (analysis_id, ply)
);

CREATE TABLE explanations (
    id                TEXT PRIMARY KEY,
    analysis_id       TEXT NOT NULL REFERENCES game_analyses(id) ON DELETE CASCADE,
    ply               INTEGER NOT NULL,
    provider          TEXT NOT NULL,
    model             TEXT NOT NULL,
    prompt_version    TEXT NOT NULL,
    request_json      TEXT NOT NULL,
    response_json     TEXT NOT NULL,
    concept_tags_json TEXT NOT NULL,
    verification_json TEXT NOT NULL,
    tokens_in         INTEGER,
    tokens_out        INTEGER,
    created_at        INTEGER NOT NULL,
    UNIQUE (analysis_id, ply)
);

-- Denormalised index of the player's errors, one row per (error, concept tag).
-- This is what "training from your mistakes" will query.
CREATE TABLE mistake_index (
    user_id        TEXT NOT NULL,
    game_id        TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    analysis_id    TEXT NOT NULL REFERENCES game_analyses(id) ON DELETE CASCADE,
    ply            INTEGER NOT NULL,
    position_id    INTEGER NOT NULL REFERENCES positions(id),
    fen            TEXT NOT NULL,
    side           TEXT NOT NULL,
    classification TEXT NOT NULL,
    phase          TEXT NOT NULL,
    concept_tag    TEXT NOT NULL,         -- '' until an explanation tags it
    delta_wc       REAL NOT NULL,
    played_uci     TEXT NOT NULL,
    best_uci       TEXT,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (analysis_id, ply, concept_tag)
);
CREATE INDEX mistakes_by_tag ON mistake_index(user_id, concept_tag);
CREATE INDEX mistakes_by_phase ON mistake_index(user_id, phase);
CREATE INDEX mistakes_by_class ON mistake_index(user_id, classification);

CREATE TABLE chat_threads (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    game_id    TEXT REFERENCES games(id) ON DELETE SET NULL,
    title      TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX chat_threads_game ON chat_threads(user_id, game_id, updated_at DESC);

CREATE TABLE chat_messages (
    id         TEXT PRIMARY KEY,
    thread_id  TEXT NOT NULL REFERENCES chat_threads(id) ON DELETE CASCADE,
    seq        INTEGER NOT NULL,
    role       TEXT NOT NULL,
    view_json  TEXT NOT NULL,             -- api_types::ChatMessage
    ir_json    TEXT NOT NULL,             -- provider-neutral LLM messages for this turn, replayed verbatim
    fen        TEXT,
    ply        INTEGER,
    tokens_in  INTEGER,
    tokens_out INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX chat_messages_thread ON chat_messages(thread_id, seq);
