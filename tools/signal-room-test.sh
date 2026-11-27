#!/usr/bin/env bash
# Runs the signaling room service (signal/, the Worker) locally with Wrangler and has
# beam-core's client join it as two devices (core/beam-core/tests/signal_room_live.rs), and
# the phone's browser pairing announce its offer there (core/beam-ffi/tests/browser_pairing.rs).
#
# Needs: `npm ci` at the repo root. PORT picks Wrangler's port (default 8787).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
port=${PORT:-8787}
. "$root/tools/lib/signal-dev.sh"
trap stop_signal_dev EXIT

(cd "$root/core" && cargo build --locked -p beam-core -p beam-ffi --tests)
start_signal_dev "$root" "$port"

cd "$root/core"
export BEAM_SIGNAL_URL="$signal_dev_url"
cargo test --locked -p beam-core --test signal_room_live -- --ignored --nocapture
cargo test --locked -p beam-ffi --test browser_pairing -- --ignored --nocapture
