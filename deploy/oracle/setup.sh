#!/usr/bin/env bash
# One-time setup on a fresh Oracle Cloud Ubuntu (ARM) server.
#   curl -fsSL https://raw.githubusercontent.com/andrewnakas/ChessGpt/main/deploy/oracle/setup.sh | bash
set -euo pipefail

REPO_RAW="${REPO_RAW:-https://raw.githubusercontent.com/andrewnakas/ChessGpt/main}"
DIR="$HOME/chessgpt"

if ! command -v docker >/dev/null; then
  echo "Installing Docker…"
  curl -fsSL https://get.docker.com | sudo sh
  sudo usermod -aG docker "$USER"
fi

mkdir -p "$DIR"
cd "$DIR"
curl -fsSL "$REPO_RAW/deploy/oracle/docker-compose.yml" -o docker-compose.yml

if [ ! -f .env ]; then
  read -rp "Cloudflare tunnel token: " TUNNEL_TOKEN
  read -rp "Docker image [ghcr.io/andrewnakas/chessgpt:latest]: " IMAGE
  read -rp "Anthropic API key for the on-site coach (optional, Enter to skip): " AKEY
  cat > .env <<ENV
TUNNEL_TOKEN=$TUNNEL_TOKEN
CHESSGPT_IMAGE=${IMAGE:-ghcr.io/andrewnakas/chessgpt:latest}
CHESSGPT_DOMAIN=chessgpt.com
ANTHROPIC_API_KEY=$AKEY
ENV
  chmod 600 .env
fi

# Update script: pulls the newest image GitHub built and restarts.
cat > update.sh <<'UPD'
#!/usr/bin/env bash
cd "$(dirname "$0")" && sudo docker compose pull && sudo docker compose up -d && sudo docker image prune -f
UPD
chmod +x update.sh

sudo docker compose pull
sudo docker compose up -d
# Check for a new image every night at 04:00.
( crontab -l 2>/dev/null | grep -v chessgpt/update.sh; echo "0 4 * * * $DIR/update.sh >/dev/null 2>&1" ) | crontab -
echo
echo "chessgpt is starting. Logs: sudo docker compose -f $DIR/docker-compose.yml logs -f"
