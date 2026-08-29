#!/usr/bin/env bash
# Chrona live demo — run the real daemon with a simulated watcher feed and
# serve the dashboard in a browser. No compositor required.
#
#   ./tools/demo/run_demo.sh              # http://localhost:3000
#   PORT=8080 ./tools/demo/run_demo.sh
#
# What's real here: chronad (watcher pipeline, SQLite storage, rule engine,
# stats, goals API). What's simulated: window events (seed.py + ticker.py).
set -euo pipefail
cd "$(dirname "$0")/../.."

RUN_DIR="${CHRONA_RUN_DIR:-/tmp/chrona-demo}"
SOCKET="$RUN_DIR/chrona.sock"
DB_DIR="$RUN_DIR/data"
PORT="${PORT:-3000}"
INTERVAL="${TICK_INTERVAL:-5}"

# Locate (or build) the daemon.
BIN="${CHRONAD:-}"
if [ -z "$BIN" ]; then
  for cand in \
    "$(pwd)/target/release/chronad" \
    "$(pwd)/target/debug/chronad" \
    "$HOME/.cargo/bin/chronad"; do
    [ -x "$cand" ] && BIN="$cand" && break
  done
fi
if [ -z "$BIN" ]; then
  echo "[demo] chronad not found — building (cargo build -p chronad)…"
  cargo build -p chronad
  BIN="$(pwd)/target/debug/chronad"
fi

mkdir -p "$DB_DIR"

# Fresh DB on first run (or after `rm -rf /tmp/chrona-demo`).
if [ ! -s "$DB_DIR/chrona.db" ]; then
  echo "[demo] seeding 50 days of realistic usage…"
  python3 tools/demo/seed.py --db "$DB_DIR/chrona.db" --days 50 --fresh
fi

cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
PIDS=()
trap cleanup EXIT INT TERM

echo "[demo] starting chronad ($BIN)"
"$BIN" --socket "$SOCKET" --data-dir "$DB_DIR" &
PIDS+=($!)

sleep 1

echo "[demo] starting simulated watcher ticker"
python3 tools/demo/ticker.py --db "$DB_DIR/chrona.db" --socket "$SOCKET" --interval "$INTERVAL" &
PIDS+=($!)

echo "[demo] dashboard: http://localhost:$PORT"
python3 tools/demo/server.py --socket "$SOCKET" --port "$PORT"
