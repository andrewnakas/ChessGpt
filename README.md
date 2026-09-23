# chessgpt

An open-source chess coach. Stockfish analyses every move of your games; a language model
explains the moments that mattered at your rating, and answers questions about any position.
Every move the model mentions is checked against the engine before you see it.

- **Game analysis** – paste a PGN or FEN, or import from Lichess and Chess.com. Every move is
  classified exactly the way Lichess does it (inaccuracy / mistake / blunder, accuracy %),
  plus book, best, excellent and missed-win labels. See [docs/classification.md](docs/classification.md).
- **Explanations at your level** – the key moments get a deeper engine pass and a structured
  explanation written for your rating: what happened, why, the better move with the engine's
  own line, and a takeaway.
- **Coach chat** – ask about the position on the board. The coach calls Stockfish, plays out
  lines to check them, reads the opening explorer, and looks up your game.
- **No hallucinated moves** – illegal or invented moves trigger a correction round, then
  are stripped; lines are replaced with the engine's. Each answer shows whether it was verified.
- **Inside Claude and ChatGPT** – people add `https://chessgpt.com/mcp` as a connector and use
  chessgpt from the assistant they already pay for: Stockfish, move checking, their library and
  an interactive board, no API key. Every result links back to the site. See
  [docs/integrations.md](docs/integrations.md).
- **No user keys on the site** – the site operator sets one model key (with a daily budget), or
  in local mode you can use Claude, OpenAI, OpenRouter, Ollama or any OpenAI-compatible server.
- **Accounts** – email and password or Sign in with Lichess (which also imports your Lichess
  games), share links for any game.

## Run it

You need Rust 1.98+ and Node 24+ (`winget install OpenJS.NodeJS.LTS` on Windows).

```sh
cargo xtask setup     # web dependencies, Stockfish 19 (checksum-pinned), generated types
cargo xtask build     # one release binary with the web UI embedded
./target/release/chessgpt
```

Open http://localhost:8080 and import a game. For the coach, start the server with
`ANTHROPIC_API_KEY` set (or add a provider under **Settings** in local mode).

For development, `cargo xtask dev` runs the API on :8080 and the Vite dev server on :5173.
If you have `just`, the `justfile` wraps the same commands.

### Configuration

All optional; see `.env.example`.

| Variable | Default | |
|---|---|---|
| `CHESSGPT_BIND` | `127.0.0.1:8080` (`0.0.0.0:8080` hosted) | listen address |
| `CHESSGPT_DATA_DIR` | `%LOCALAPPDATA%\chessgpt` / `~/.local/share/chessgpt` | database and key file |
| `STOCKFISH_PATH` | `engines/…` | Stockfish binary |
| `ENGINE_WORKERS`, `ENGINE_THREADS`, `ENGINE_HASH_MB` | from CPU count | engine pool size |
| `CHESSGPT_MODE` | `local` | `hosted` turns on accounts, OAuth and the connector |
| `CHESSGPT_PUBLIC_URL` | | e.g. `https://chessgpt.com`; required for the connector |
| `ANTHROPIC_API_KEY` or `CHESSGPT_LLM_*` | | the site's own model; hides key settings from users |
| `CHESSGPT_LLM_DAILY_TOKENS` | unlimited | daily token cap for the site's model |

API keys are stored encrypted with a per-install key (`keyring.key` in the data directory).

### Lichess

Since August 2026 Lichess only serves a player's game list and the opening explorer to
signed-in API clients ([lichess-org/api#667](https://github.com/lichess-org/api/issues/667)).
Signing in to chessgpt with Lichess covers both. In local mode, paste a personal token (no
scopes) from https://lichess.org/account/oauth/token in Settings. Single games import by link
without a token.

## Deploy

`deploy/` has a Dockerfile (web build, Rust build, Stockfish download, slim runtime), a
Caddyfile with automatic HTTPS, and a compose file:

```sh
cd deploy
ANTHROPIC_API_KEY=... CHESSGPT_LLM_DAILY_TOKENS=5000000 docker compose up -d --build
```

## Layout

| Path | |
|---|---|
| `crates/chess-core` | win %, Lichess judgements and accuracy, PGN, opening book |
| `crates/engine` | Stockfish process pool: priorities, cancellation, caching |
| `crates/llm` | Anthropic and OpenAI-compatible streaming clients, tool calls |
| `crates/coach` | game analysis pipeline, prompts, move verifier, chat tools |
| `crates/importers` | Lichess and Chess.com |
| `crates/db` | SQLite schema and queries |
| `crates/api-types` | every API type; generates `web/src/lib/api/types.ts` |
| `crates/server` | the `chessgpt` binary |
| `web` | SvelteKit UI (chessground board) |

## Tests

```sh
cargo test --workspace     # engine tests use the Stockfish in engines/
cd web && npm test && npm run check
```

## License

chessgpt is AGPL-3.0-only. It runs [Stockfish](https://github.com/official-stockfish/Stockfish)
(GPL-3.0) as a separate program. Board: [chessground](https://github.com/lichess-org/chessground),
rules: [chessops](https://github.com/niklasf/chessops) and
[shakmaty](https://github.com/niklasf/shakmaty) (GPL-3.0-or-later). Opening names:
[lichess-org/chess-openings](https://github.com/lichess-org/chess-openings) (CC0).
