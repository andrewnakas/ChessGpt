# Rating model data

`chess_core::rating::COEF` was fitted with `chessgpt-lab fit-rating` on the 3,648 games carrying
`[%eval]` on every move among the first 80 MB of the Lichess database for August 2026
(https://database.lichess.org/, CC0): 7,235 rated sides, with every fifth held out.

Held-out mean absolute error for a single game is 317 rating points, against 369 for always
predicting the average. One game says little, so the site shows a rolling mean of the last ten
games. When the players' ratings are known, it prefers the performance rating
(`rating::performance`).

To refit:

    curl -sL https://database.lichess.org/standard/lichess_db_standard_rated_2026-08.pgn.zst \
      | zstd -dc | head -c 80000000 > head.pgn
    # keep games whose moves all have [%eval], then:
    cargo run -p lab -- fit-rating eval_games.pgn
