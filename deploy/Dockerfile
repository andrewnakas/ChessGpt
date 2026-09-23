# syntax=docker/dockerfile:1
# Build from the repo root:  docker build -f deploy/Dockerfile -t chessgpt .

FROM node:24-bookworm AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
# The build also copies the browser engine and the opening book into the site.
COPY crates/chess-core/data/openings /src/crates/chess-core/data/openings
RUN npm run build

FROM rust:1.98-bookworm AS server
WORKDIR /src
COPY . .
COPY --from=web /src/web/build web/build
RUN cargo build --release -p server --locked

FROM debian:bookworm-slim
# Set automatically by docker buildx: amd64 or arm64 (Oracle Ampere).
ARG TARGETARCH
ARG SF_TAG=sf_19
RUN set -eu;     case "$TARGETARCH" in       amd64) SF=stockfish-linux-x86-64-universal; SHA=9defc0d4e55d49c65a6d042f3e571a39fcea499ade6dbe741b53b8c65e03611f ;;       arm64) SF=stockfish-linux-arm64-universal;  SHA=fe26cfd1d9db4c8af3d21e24d9ff34cacb31c1f940085a7583da11796f2bac01 ;;       *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;;     esac;     apt-get update;     apt-get install -y --no-install-recommends ca-certificates curl;     curl -fsSL -o /tmp/sf.tar.gz "https://github.com/official-stockfish/Stockfish/releases/download/${SF_TAG}/${SF}.tar.gz";     echo "${SHA}  /tmp/sf.tar.gz" | sha256sum -c -;     tar -xzf /tmp/sf.tar.gz -C /opt;     mv "/opt/stockfish/${SF}" /opt/stockfish/stockfish;     rm -rf /tmp/sf.tar.gz /opt/stockfish/src /opt/stockfish/wiki /var/lib/apt/lists/*;     useradd --system --home /data --uid 10001 chessgpt;     mkdir -p /data; chown chessgpt /data
COPY --from=server /src/target/release/chessgpt /usr/local/bin/chessgpt
USER chessgpt
ENV CHESSGPT_BIND=0.0.0.0:8080 \
    CHESSGPT_DATA_DIR=/data \
    STOCKFISH_PATH=/opt/stockfish/stockfish \
    CHESSGPT_MODE=hosted
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s CMD curl -fsS http://127.0.0.1:8080/api/health || exit 1
CMD ["chessgpt"]
