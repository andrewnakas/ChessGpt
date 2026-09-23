-- Estimated playing strength per side ("played like"), from move quality.
ALTER TABLE game_analyses ADD COLUMN white_estimate INTEGER;
ALTER TABLE game_analyses ADD COLUMN black_estimate INTEGER;

-- How a mistake's tag was found: 'missed' (the better line used it),
-- 'allowed' (the opponent's reply uses it), or 'coach' (the explanation).
ALTER TABLE mistake_index ADD COLUMN motif_kind TEXT NOT NULL DEFAULT '';
