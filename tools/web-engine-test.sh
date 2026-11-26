#!/usr/bin/env bash
# Builds the WebAssembly engine and the native test peers, then pairs them in headless Chromium
# (core/beam-web/e2e/handshake.e2e.mjs), the peer sends files into the engine's worker
# (receive.e2e.mjs) and receives files picked in the page (send.e2e.mjs), and the phone app's
# engine pairs with the browser (phone-pairing.e2e.mjs). Then runs beam-web's browser-only Rust
# tests (core/beam-web/tests, such as OPFS staging in a dedicated worker) in the same Chromium.
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

# wasm-bindgen (and its test runner) refuse a module built by a different version of its crate.
wanted=$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')
have=$(wasm-bindgen --version | awk '{print $2}')
if [ "$wanted" != "$have" ]; then
  echo "wasm-bindgen-cli is $have but core/Cargo.lock has $wanted:" >&2
  echo "  cargo install --locked wasm-bindgen-cli --version $wanted" >&2
  exit 1
fi

cargo build --locked -p beam-core --example webtransport_peer
cargo build --locked -p beam-ffi --example browser_pairing_peer
cargo build --locked -p beam-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$target/beam-web-pkg" \
  "$target/wasm32-unknown-unknown/release/beam_web.wasm"

node "$root/core/beam-web/e2e/handshake.e2e.mjs" \
  "$target/beam-web-pkg" "$target/debug/examples/webtransport_peer"
node "$root/core/beam-web/e2e/receive.e2e.mjs" \
  "$target/beam-web-pkg" "$target/debug/examples/webtransport_peer"
node "$root/core/beam-web/e2e/send.e2e.mjs" \
  "$target/beam-web-pkg" "$target/debug/examples/webtransport_peer"
node "$root/core/beam-web/e2e/phone-pairing.e2e.mjs" \
  "$target/beam-web-pkg" "$target/debug/examples/browser_pairing_peer"

# Integration tests only: the unit tests are host tests that `cargo test` already runs.
browser_tests=$(cargo test --locked -p beam-web --target wasm32-unknown-unknown --no-run \
  --message-format=json | grep '"kind":\["test"\]' | sed -n 's/.*"executable":"\([^"]*\)".*/\1/p')
if [ -z "$browser_tests" ]; then
  echo "no browser tests were built" >&2
  exit 1
fi
for tests in $browser_tests; do
  node "$root/core/beam-web/e2e/wasm-tests.e2e.mjs" "$tests"
done
