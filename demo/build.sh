#!/usr/bin/env sh
# Builds the demo into site/public/demo/, where the website serves it from.
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version 0.2.129
set -e
cd "$(dirname "$0")"
cargo build --release
wasm-bindgen --target web --no-typescript --out-dir ../site/public/demo \
  target/wasm32-unknown-unknown/release/spotini_demo.wasm
