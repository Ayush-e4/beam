#!/usr/bin/env bash
# Runs the signaling room service (signal/, the Worker) locally with Wrangler and has
# beam-core's client join it as two devices (core/beam-core/tests/signal_room_live.rs).
#
# Needs: `npm ci` at the repo root. PORT picks Wrangler's port (default 8787).
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
port=${PORT:-8787}
log=$(mktemp)

(cd "$root/signal" && exec npx wrangler dev --port "$port" --ip 127.0.0.1 --log-level warn) \
  >"$log" 2>&1 &
wrangler=$!
trap 'kill "$wrangler" 2>/dev/null || true; rm -f "$log"' EXIT

(cd "$root/core" && cargo build --locked -p beam-core --tests)

for _ in $(seq 60); do
  if curl -s -o /dev/null "http://127.0.0.1:$port/"; then break; fi
  if ! kill -0 "$wrangler" 2>/dev/null; then cat "$log" >&2; exit 1; fi
  sleep 1
done

cd "$root/core"
BEAM_SIGNAL_URL="ws://127.0.0.1:$port" \
  cargo test --locked -p beam-core --test signal_room_live -- --ignored --nocapture
