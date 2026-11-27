# Sourced by the test scripts: runs the signaling service (signal/, the Worker) locally with
# Wrangler on 127.0.0.1:$1 until the script exits, and waits until it answers. Needs `npm ci`
# at the repo root. Sets `signal_dev_url` (ws://127.0.0.1:$1).
#
#   . "$root/tools/lib/signal-dev.sh"
#   start_signal_dev "$root" 8787
#   stop_signal_dev   # from the script's EXIT trap

start_signal_dev() {
  local root=$1 port=$2
  signal_dev_log=$(mktemp)
  (cd "$root/signal" && WRANGLER_SEND_METRICS=false \
    exec npx wrangler dev --port "$port" --ip 127.0.0.1 --log-level warn) \
    >"$signal_dev_log" 2>&1 &
  signal_dev_pid=$!
  for _ in $(seq 60); do
    if curl -s -o /dev/null "http://127.0.0.1:$port/"; then
      signal_dev_url="ws://127.0.0.1:$port"
      return 0
    fi
    if ! kill -0 "$signal_dev_pid" 2>/dev/null; then break; fi
    sleep 1
  done
  echo "the signaling service didn't start:" >&2
  cat "$signal_dev_log" >&2
  return 1
}

stop_signal_dev() {
  if [ -n "${signal_dev_pid:-}" ]; then kill "$signal_dev_pid" 2>/dev/null || true; fi
  if [ -n "${signal_dev_log:-}" ]; then rm -f "$signal_dev_log"; fi
}
