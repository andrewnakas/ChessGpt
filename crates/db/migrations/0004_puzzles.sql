-- Training puzzles with spaced-repetition state.
CREATE TABLE puzzles (
    id            TEXT PRIMARY KEY,
    user_id       TEXT NOT NULL,
    source        TEXT NOT NULL,          -- 'mistake' (the user's own games)
    game_id       TEXT REFERENCES games(id) ON DELETE CASCADE,
    analysis_id   TEXT,
    ply           INTEGER,
    fen           TEXT NOT NULL,
    solution_json TEXT NOT NULL,          -- UCI moves
    line_json     TEXT NOT NULL,          -- SAN engine line, shown after solving
    themes_json   TEXT NOT NULL,          -- motif tags
    interval_days REAL NOT NULL DEFAULT 0,
    ease          REAL NOT NULL DEFAULT 2.5,
    reps          INTEGER NOT NULL DEFAULT 0,
    lapses        INTEGER NOT NULL DEFAULT 0,
    attempts      INTEGER NOT NULL DEFAULT 0,
    due_at        INTEGER NOT NULL,
    created_at    INTEGER NOT NULL
);
CREATE UNIQUE INDEX puzzles_source ON puzzles(user_id, game_id, ply);
CREATE INDEX puzzles_due ON puzzles(user_id, due_at);
