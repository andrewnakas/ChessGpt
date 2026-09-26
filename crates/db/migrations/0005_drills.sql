-- Technique drill sets: one concept trained several ways.
CREATE TABLE drill_sets (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL,
    technique    TEXT NOT NULL,          -- concept tag
    source_json  TEXT NOT NULL,          -- api_types::DrillSource
    rating       INTEGER NOT NULL,       -- game rating the set was pitched at
    created_at   INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX drill_sets_user ON drill_sets(user_id, created_at);

CREATE TABLE drill_items (
    id           TEXT PRIMARY KEY,
    set_id       TEXT NOT NULL REFERENCES drill_sets(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL,
    pos          INTEGER NOT NULL,       -- order in the set
    kind         TEXT NOT NULL,          -- spot | find | defend | playout
    bank_id      TEXT,                   -- Lichess puzzle id for bank items
    item_json    TEXT NOT NULL,          -- api_types::DrillItem
    solved       INTEGER,                -- NULL until attempted
    ms           INTEGER,
    hints_used   INTEGER,
    attempted_at INTEGER
);
CREATE INDEX drill_items_set ON drill_items(set_id, pos);
CREATE INDEX drill_items_user ON drill_items(user_id, attempted_at);

-- Defence puzzles accept any of several first moves.
ALTER TABLE puzzles ADD COLUMN accept_json TEXT;
-- Drill misses become review puzzles once per position.
CREATE UNIQUE INDEX puzzles_drill ON puzzles(user_id, fen) WHERE source = 'drill';
