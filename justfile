# Thin wrappers around `cargo xtask` (works without just, too).

default:
    @just --list

setup:
    cargo xtask setup

dev:
    cargo xtask dev

build:
    cargo xtask build

run: build
    ./target/release/chessgpt

test:
    cargo test --workspace
    cd web && npm test && npm run check

# OAuth + MCP end-to-end against a hosted-mode server on :18081
e2e:
    python tools/mcp_e2e.py http://127.0.0.1:18081

types:
    cargo xtask gen-types

stockfish:
    cargo xtask fetch-stockfish
