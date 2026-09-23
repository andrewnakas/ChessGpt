# chessgpt in Claude and ChatGPT

chessgpt is a remote MCP server at `https://<your-domain>/mcp`. People add it to the Claude or
ChatGPT they already use. Their assistant does the reasoning on their own subscription;
chessgpt supplies Stockfish, move checking, their game library, and an interactive board.
Nobody needs an API key.

Everything the assistant analyses is saved to the user's chessgpt account, and every result
links back to the site (`/analyse/<game>?ply=<n>`, `/board?fen=…`).

## How it fits together

| Piece | Where |
|---|---|
| MCP server (Streamable HTTP, stateless, protocol `2026-07-28` and earlier) | `crates/server/src/mcp/mod.rs`, built on `rmcp` |
| Board widget (MCP Apps, `text/html;profile=mcp-app`) | `crates/server/src/mcp/widget.rs` |
| OAuth 2.1 authorization server | `crates/server/src/oauth.rs` |
| Accounts (email + password, Sign in with Lichess), sessions, share links | `crates/server/src/auth.rs`, `crates/db/src/accounts.rs` |
| Claude Code plugin + coaching skill | `integrations/claude-plugin/`, marketplace in `.claude-plugin/` |
| End-to-end test (OAuth + MCP, both protocol eras) | `tools/mcp_e2e.py` |

### Tools

| Tool | Read-only | What it does |
|---|---|---|
| `analyze_position` | yes | Stockfish lines for a FEN, with the board widget |
| `check_line` | yes | Plays SAN moves, rejects illegal ones, evaluates the end position |
| `legal_moves` | yes | Legal moves, checks and captures |
| `opening_explorer` | yes | Lichess explorer (needs a Lichess sign-in) |
| `analyze_game` | no (adds to library) | Saves a PGN or Lichess game and classifies every move |
| `game_report` | yes | Stored analysis: accuracy, key moments, better lines |
| `my_games` | yes | Recent games in the library |
| `my_weaknesses` | yes | Mistakes by theme and phase |
| `import_games` | no (adds to library) | Recent games from Lichess or Chess.com |

Every tool has a `title` and `readOnlyHint`/`destructiveHint`/`openWorldHint` annotations, as
both directories require.

### Sign-in

`/.well-known/oauth-protected-resource/mcp` points clients to chessgpt's own authorization
server (`/.well-known/oauth-authorization-server`). It supports Client ID Metadata Documents
(what Claude and ChatGPT prefer) and dynamic client registration, PKCE S256, RFC 8707
resource binding, RFC 9207 `iss`, and rotating refresh tokens. Users sign in to chessgpt with
email and password or with Lichess, then approve the app on a consent screen. They can
disconnect apps from `/connect`.

In local mode (`CHESSGPT_MODE=local`) there are no accounts and `/mcp` is open to the local
user, which is handy for Claude Desktop or Claude Code on the same machine.

## Running it publicly

```sh
CHESSGPT_MODE=hosted
CHESSGPT_PUBLIC_URL=https://chessgpt.com     # required: issuer, resource URL and Host allowlist
```

- Serve over HTTPS (the Caddy config in `deploy/` does this).
- Claude connects from `160.79.104.0/21`; don't firewall it.
- Lichess sign-in needs no registration; the redirect is `https://<domain>/auth/lichess/callback`.

## Adding it yourself

**Claude** (Free allows one custom connector; Pro, Max, Team, Enterprise): Settings → Connectors
→ Add custom connector → `https://chessgpt.com/mcp` → Connect.

**ChatGPT** (Plus, Pro, Business, Enterprise, Edu on the web): Settings → Security → Developer
mode on, then Settings → Apps → Create → URL `https://chessgpt.com/mcp`, authentication OAuth.

**Claude Code**: `/plugin marketplace add chessgpt/chessgpt` then `/plugin install chessgpt@chessgpt`,
or `claude mcp add --transport http chessgpt https://chessgpt.com/mcp`.

## Getting listed

### Anthropic Connectors Directory

Submit from a Claude Team or Enterprise organization (Owner) at
claude.ai → Admin settings → Directory → Submissions. You'll need:

- [x] `title` and `readOnlyHint`/`destructiveHint` on every tool, names ≤ 64 characters
- [x] OAuth 2.0 sign-in
- [ ] A test account with analysed games (create one on the live site)
- [ ] Public documentation URL (`https://chessgpt.com/connect`)
- [ ] Privacy policy URL (not written yet)
- [ ] 3–5 PNG screenshots of the board widget in Claude, at least 1000 px wide

### ChatGPT plugin directory

Submit at platform.openai.com/plugins. You'll need:

- [ ] Individual or business verification
- [ ] Domain verification: set `CHESSGPT_OPENAI_CHALLENGE` to the token OpenAI gives you; it is
      served at `/.well-known/openai-apps-challenge`
- [x] `readOnlyHint`, `openWorldHint`, `destructiveHint` on every tool
- [x] A content-security policy for the widget (it makes no network requests)
- [ ] Privacy, terms and support URLs
- [ ] A demo account, 5 positive and 3 negative test prompts
- Selling digital goods inside ChatGPT is not allowed; linking an existing chessgpt account is.

## Testing

```sh
CHESSGPT_MODE=hosted CHESSGPT_PUBLIC_URL=http://127.0.0.1:18081 CHESSGPT_BIND=127.0.0.1:18081 \
  CHESSGPT_DATA_DIR=/tmp/cg ./target/release/chessgpt &
python tools/mcp_e2e.py http://127.0.0.1:18081
```

It registers a client, signs up, approves, exchanges the code with PKCE, rotates the refresh
token, then runs the tools over both the `2025-11-25` handshake and `2026-07-28` per-request
metadata, and checks that the analysed game appears on the website for the same account only.

Not yet verified against the live Claude and ChatGPT apps: that needs the server on a public
HTTPS domain.
