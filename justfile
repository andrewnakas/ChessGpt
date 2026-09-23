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

types:
    cargo xtask gen-types

stockfish:
    cargo xtask fetch-stockfish
