#!/usr/bin/env bash
# The real clipboard test between a phone and Chromium on this Mac, over the Wi-Fi they share:
# installs android/core's instrumented tests on the phone, builds the browser engine, runs the
# signaling service (signal/) under Wrangler for both to meet in (the phone reaches it through
# `adb reverse`), and runs core/beam-web/e2e/phone-clipboard.e2e.mjs, which drives both sides.
#
# Needs: one phone on adb with its screen on and unlocked, on the same Wi-Fi as the Mac (a
# network that lets devices reach each other); what tools/web-engine-test.sh needs for the
# browser engine; `npm ci` at the repo root; and JAVA_HOME/ANDROID_HOME for Gradle (see
# android/README). PORT picks the signaling port (default 8787).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
target=${CARGO_TARGET_DIR:-$root/core/target}
adb=${ADB:-${ANDROID_HOME:-$HOME/Library/Android/sdk}/platform-tools/adb}
port=${PORT:-8787}
. "$root/tools/lib/signal-dev.sh"
trap '"$adb" reverse --remove "tcp:$port" >/dev/null 2>&1; stop_signal_dev' EXIT

(cd "$root/android" && ./gradlew -q :core:installDebugAndroidTest -Pbeam.rustAbis=arm64-v8a)

cd "$root/core"
cargo build --locked -p beam-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$target/beam-web-pkg" \
  "$target/wasm32-unknown-unknown/release/beam_web.wasm"

start_signal_dev "$root" "$port"
"$adb" reverse "tcp:$port" "tcp:$port" >/dev/null

node "$root/core/beam-web/e2e/phone-clipboard.e2e.mjs" "$target/beam-web-pkg" "$adb" \
  "$signal_dev_url"
