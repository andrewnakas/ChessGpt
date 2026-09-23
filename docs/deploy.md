# Putting chessgpt.com online (free)

```
visitor ──► Cloudflare (chessgpt.com)
              ├─ web app: served by Cloudflare itself, always up
              └─ /api /mcp /oauth /auth ──► Cloudflare Tunnel ──► Oracle Cloud server (free, ARM)
                                              │
                         server down? ────────┘  the app switches to browser mode:
                                                 Stockfish in the visitor's browser
```

- **Oracle Cloud Always Free** runs the real server: Stockfish, database, accounts, the coach,
  and the Claude/ChatGPT connector. 2 ARM cores and 12 GB RAM, free.
- **Cloudflare** serves the site and the domain, and tunnels to Oracle (no open ports).
- **GitHub** builds everything for free on every push.
- If Oracle is down, chessgpt.com still works in **browser mode**: analysis runs in the visitor's
  browser and games are kept on their device. The coach, accounts and the connector come back
  with the server.

You need: the domain on Cloudflare (nameservers moved from Namecheap), a GitHub account, an
Oracle Cloud account.

## 1. Put the code on GitHub

1. The code is public at https://github.com/andrewnakas/ChessGpt. On another machine:
   `git clone https://github.com/andrewnakas/ChessGpt.git chessgpt`
2. Every push to `main` runs the build.
3. Watch **Actions → release**. It builds the server for Intel and ARM (about 15 minutes the
   first time) and publishes `ghcr.io/andrewnakas/chessgpt`.
4. When it's done: your GitHub profile → **Packages → chessgpt → Package settings → Change
   visibility → Public**, so the Oracle server can download it without a password.

## 2. Create the Cloudflare Tunnel

1. Cloudflare dashboard → **Zero Trust → Networks → Tunnels → Create a tunnel → Cloudflared**,
   name it `chessgpt`.
2. On the install step, pick **Docker** and copy the long token after `--token` (you'll paste it
   on the server; don't run the command here).
3. **Public hostname**: subdomain `origin`, domain `chessgpt.com`, type `HTTP`, URL
   `chessgpt:8080`. Save.

## 3. Create the Oracle server

1. Sign up at cloud.oracle.com (a card is needed to verify you; Always Free resources aren't
   charged). Pick a home region close to you; if it later says "out of capacity", try another
   availability domain or try again later.
2. **Compute → Instances → Create instance**:
   - Image: **Canonical Ubuntu 24.04**
   - Shape: **Ampere → VM.Standard.A1.Flex**, **2 OCPUs, 12 GB memory** (the Always Free limit)
   - Add your SSH key (or let it generate one and download it)
   - Create.
3. No firewall changes are needed: the tunnel connects outwards.

## 4. Start chessgpt on the server

SSH in (`ssh ubuntu@<public-ip>`) and run:

```
curl -fsSL https://raw.githubusercontent.com/andrewnakas/ChessGpt/main/deploy/oracle/setup.sh -o setup.sh
REPO_RAW=https://raw.githubusercontent.com/andrewnakas/ChessGpt/main bash setup.sh
```

It installs Docker and asks for:
- the **tunnel token** from step 2,
- the image: `ghcr.io/andrewnakas/chessgpt:latest`,
- optionally an **Anthropic API key** for the on-site coach (capped at 3M tokens a day; change
  `CHESSGPT_LLM_DAILY_TOKENS` in `~/chessgpt/.env`).

It starts chessgpt plus the tunnel and installs a nightly update from GitHub. Check:
`https://origin.chessgpt.com/api/health` should say `ok`.

## 5. Put the web app on Cloudflare

1. Cloudflare dashboard → **My Profile → API Tokens → Create Token → "Edit Cloudflare
   Workers"** template → include your account and the chessgpt.com zone → create, copy it.
2. Your Cloudflare **Account ID** is on the right of the dashboard home page.
3. GitHub repo → **Settings → Secrets and variables → Actions → New repository secret**:
   `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`.
4. **Actions → release → Run workflow**. The `cloudflare` job deploys the site and claims
   `chessgpt.com` and `www.chessgpt.com`. (Delete any old A/CNAME records for `@` and `www` in
   Cloudflare DNS first, or the domain claim fails.)

**Or deploy from your Mac** with Wrangler (already logged in to Cloudflare):

```
git clone https://github.com/andrewnakas/ChessGpt.git chessgpt && cd chessgpt
cd web && npm ci && npm run build && cd ..
npx wrangler deploy --config deploy/cloudflare/wrangler.jsonc
```

## 6. Check it

- https://chessgpt.com shows chessgpt; create your account (or sign in with Lichess).
- https://chessgpt.com/.well-known/oauth-protected-resource/mcp returns JSON.
- Claude → Settings → Connectors → Add custom connector → `https://chessgpt.com/mcp`.
- Fallback test: on the server run `cd ~/chessgpt && sudo docker compose stop chessgpt`, reload
  chessgpt.com: a "Browser mode" banner appears and analysis still works. Start it again with
  `sudo docker compose start chessgpt`.

In Cloudflare, keep **Security → Bots → Bot Fight Mode** off: Claude and ChatGPT call `/mcp`
from their servers.

## Updating

Push to `main`. GitHub rebuilds the image and redeploys the site; the server picks up the new
image at 04:00, or right away with `~/chessgpt/update.sh`.

## Good to know

- Oracle can change or reclaim free resources (it halved the free ARM size in June 2026).
  Nothing is lost for visitors: the site falls back to browser mode. Back up the database
  volume now and then:
  `sudo docker run --rm -v chessgpt_chessgpt-data:/d -v $PWD:/b alpine tar czf /b/chessgpt-backup.tgz -C /d .`
- Cloudflare's free Workers plan allows 100,000 requests a day through the edge proxy; page
  files don't count against it.
- The server's database lives in the `chessgpt-data` Docker volume on the Oracle disk.
