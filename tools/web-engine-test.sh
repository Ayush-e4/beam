#!/usr/bin/env bash
# Builds the WebAssembly engine and the native test peer, then pairs them in headless Chromium
# (core/beam-web/e2e/handshake.e2e.mjs).
#
# Needs: the wasm32-unknown-unknown target (rust-toolchain.toml), wasm-bindgen-cli at the same
# version as the wasm-bindgen crate in core/Cargo.lock, a clang that targets wasm
# (CC_wasm32_unknown_unknown / AR_wasm32_unknown_unknown; Apple's clang doesn't, the Android
# NDK's does), `npm ci` at the repo root, and Chromium (`npx playwright-core install chromium`,
# or CHROMIUM_PATH=/path/to/chrome).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/core/target}
cd "$root/core"

# wasm-bindgen refuses a module built by a different version of its crate.
wanted=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
have=$(wasm-bindgen --version | awk '{print $2}')
if [ "$wanted" != "$have" ]; then
  echo "wasm-bindgen-cli is $have but core/Cargo.lock has $wanted:" >&2
  echo "  cargo install --locked wasm-bindgen-cli --version $wanted" >&2
  exit 1
fi

cargo build --locked -p beam-core --example webtransport_peer
cargo build --locked -p beam-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$target/beam-web-pkg" \
  "$target/wasm32-unknown-unknown/release/beam_web.wasm"

node "$root/core/beam-web/e2e/handshake.e2e.mjs" \
  "$target/beam-web-pkg" "$target/debug/examples/webtransport_peer"
