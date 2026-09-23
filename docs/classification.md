# How moves are judged

chessgpt reproduces Lichess's analysis exactly, then adds a few labels on top.

## Win percentage

Stockfish's centipawn score `cp` (White's point of view) becomes White's winning chances:

```
winning_chances(cp) = 2 / (1 + exp(-0.00368208 * cp)) - 1      (−1 … +1)
win%                = 50 + 50 * winning_chances(clamp(cp, -1000, 1000))
```

Mate scores count as ±1000 cp. Source: scalachess `WinPercent`.

## Lichess judgements (exact)

From lila `modules/tree/src/main/Advice.scala`. For the player who moved, compare the
winning chances of the position before and after the move (raw cp, no ceiling):

| Drop in winning chances (−1…1 scale) | Judgement |
|---|---|
| ≥ 0.30 | Blunder |
| ≥ 0.20 | Mistake |
| ≥ 0.10 | Inaccuracy |

Mate transitions:
- **Mate created** (the mover now gets mated): Inaccuracy if they were already below −999 cp, Mistake below −700, otherwise Blunder.
- **Mate lost** (the mover had a forced mate and lost it): Inaccuracy if still above +999 cp, Mistake above +700, otherwise Blunder.
- A slower mate is never a judgement.

`crates/chess-core/tests/lichess_regression.rs` feeds Lichess's own `[%eval]` values from
analysed games in `fixtures/lichess/` through this code and requires every `?!`, `?`
and `??` Lichess printed to be reproduced exactly. Add more analysed games there
(`https://lichess.org/game/export/<id>?evals=true&literate=true`) to widen the check.

Our own engine runs at a different depth than Lichess's server, so the same game analysed
by chessgpt can differ on borderline moves.

## chessgpt's labels

Applied in this order:

| Label | Rule |
|---|---|
| Book | Every move so far reached a named position in lichess-org/chess-openings, and Lichess would not flag it |
| Missed win | A Lichess Mistake or Blunder by a player who had ≥ 85% (or a forced mate) and dropped below 85% |
| Inaccuracy / Mistake / Blunder | The Lichess judgement |
| Best | The engine's first choice |
| Excellent | Loses less than 0.02 winning chances (about one win-percentage point) |
| Good | Everything else |

The underlying Lichess judgement is stored with every move, so "missed win" moves still
count toward Lichess-comparable totals.

## Accuracy

Per move: `103.1668 * exp(-0.04354 * (winBefore - winAfter)) - 3.1669 + 1`, clamped to
0…100, and 100 when the move did not lose anything. Per game: the average of a
volatility-weighted mean and a harmonic mean of the player's move accuracies, exactly as in
lila `AccuracyPercent.scala`.

## Key moments

The coach explains at most 8 moments per game, picked by severity × drop in winning chances
(×1.5 for your own moves), plus the turning point (after which the loser never got back
above 50%) and the largest swing in each phase. Errors made from an already lost position
(below 10%) are skipped, and two errors by the same player within two plies count once.
