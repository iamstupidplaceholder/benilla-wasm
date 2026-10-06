#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "$0")"

export RUSTUP_HOME=../../.toolchains/benilla-rustup
export CARGO_HOME=../../.toolchains/benilla-cargo

echo "Building benilla..."
"$CARGO_HOME/bin/cargo" build \
--message-format=short \
--target wasm32-unknown-unknown \
--profile wasm-dev \
-p benilla \
--no-default-features

echo "Generating wasm bindings..."
.tools/wasm-bindgen/bin/wasm-bindgen \
target/wasm32-unknown-unknown/wasm-dev/benilla.wasm \
--target web \
--out-dir web/pkg-full \
--out-name benilla \
--no-typescript

echo "Building gateway..."
"$CARGO_HOME/bin/cargo" build \
--message-format=short \
-p benilla-asset-gateway

echo "Starting gateway..."
exec target/debug/benilla-asset-gateway \
--data ../../WoW/Data \
--web web \
--auth-upstream 192.168.0.36:3724 \
--world-upstream 192.168.0.36:8085 \
"$@"
