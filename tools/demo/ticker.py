#!/usr/bin/env python3
"""Chrona live-demo ticker.

Simulates the watcher feed for the demo: keeps one "current window" event
open in SQLite (extending its end every tick) and rotates through a
plausible app pool with occasional AFK gaps. Runs alongside chronad — WAL
mode makes the daemon see every tick immediately, so the dashboard is
genuinely live, computed by the real aggregation code.

Also (idempotently) seeds a few demo goals through the real socket API and
publishes the simulated current window as the `demo.current_window`
setting, which the demo web UI reads for the "Right now" line.

Usage:
    python3 tools/demo/ticker.py --db DB --socket SOCK [--interval 5]
"""

import argparse
import json
import os
import random
import socket as syssock
import sqlite3
import sys
import time

from seed import TITLES

POOL = [
    ("firefox", 0.20),
    ("code", 0.16),
    ("konsole", 0.08),
    ("spotify", 0.13),
    ("discord", 0.11),
    ("telegram-desktop", 0.07),
    ("brave", 0.09),
    ("steam", 0.05),
    ("krita", 0.04),
    ("figma", 0.04),
    ("kate", 0.03),
    ("libreoffice-writer", 0.03),
]

DEMO_GOALS = [
    ("total", "total", 18000),
    ("category", "media", 7200),
    ("category", "gaming", 5400),
    ("app", "firefox", 10800),
    ("app", "discord", 5400),
    ("category", "communication", 3600),
]


class ChronaSock:
    def __init__(self, path):
        self.path = path

    def call(self, cmd, args=None):
        try:
            s = syssock.socket(syssock.AF_UNIX, syssock.SOCK_STREAM)
            s.settimeout(3)
            s.connect(self.path)
            s.sendall((json.dumps({"id": 1, "cmd": cmd, "args": args or {}}) + "\n").encode())
            buf = b""
            while not buf.endswith(b"\n"):
                chunk = s.recv(65536)
                if not chunk:
                    break
                buf += chunk
            s.close()
            return json.loads(buf)
        except Exception:
            return None


def pick(rng):
    r = rng.random()
    acc = 0.0
    for app, w in POOL:
        acc += w
        if r <= acc:
            return app
    return "firefox"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--socket", required=True)
    ap.add_argument("--interval", type=float, default=5.0)
    # The ticker writes FAKE events straight into SQLite, bypassing the
    # daemon's state machine. Pointing it at a real chrona.db would poison
    # genuine history — so refuse any path that does not scream "demo"
    # unless the operator explicitly passes --force.
    ap.add_argument(
        "--force",
        action="store_true",
        help="allow running against a database whose path does not contain 'demo'",
    )
    args = ap.parse_args()

    if "demo" not in os.path.abspath(args.db).lower() and not args.force:
        sys.exit(
            f"refusing to write simulated events into {args.db} (path lacks 'demo')\n"
            "the ticker fabricates window events directly in SQLite — use a dedicated demo\n"
            "database path, or pass --force if you really know what you are doing"
        )

    rng = random.Random()
    rpc = ChronaSock(args.socket)

    # Idempotent demo goals through the real API.
    res = rpc.call("goals")
    if res and res.get("ok") and not res.get("data"):
        for kind, key, limit in DEMO_GOALS:
            rpc.call("goal.set", {"kind": kind, "key": key, "limit_seconds": limit, "enabled": True})
        print("seeded demo goals via socket API")

    conn = sqlite3.connect(args.db, timeout=10)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.execute("PRAGMA busy_timeout=5000")

    cur_id = None
    cur_app = None
    cur_title = None
    rotate_at = 0
    pending_start = 0  # > now while "AFK"

    print("ticker running — feeding simulated window events")
    while True:
        now = int(time.time())
        # Honour the daemon's pause switch: the state machine ignores real
        # watcher events while paused, and fake ones must not sneak past it.
        paused = False
        pres = rpc.call("settings.get", {"key": "paused"})
        if pres and pres.get("ok") and pres.get("data", {}).get("value") == "1":
            paused = True
        if paused:
            if cur_id is not None:
                conn.execute("UPDATE events SET end = ? WHERE id = ?", (now, cur_id))
                conn.commit()
                cur_id = None
            rpc.call("settings.set", {"key": "demo.current_window", "value": "recording paused"})
            time.sleep(args.interval)
            continue
        if pending_start and now < pending_start:
            rpc.call("settings.set", {"key": "demo.current_window", "value": "AFK — away from keyboard"})
            time.sleep(args.interval)
            continue
        if pending_start:
            pending_start = 0

        if cur_id is None or now >= rotate_at:
            # close the previous event
            if cur_id is not None:
                conn.execute("UPDATE events SET end = ? WHERE id = ?", (now, cur_id))
                conn.commit()
                cur_id = None
                # occasional AFK gap -> shows up as an "unlock"
                if rng.random() < 0.22:
                    gap = rng.randint(6, 14) * 60
                    conn.execute("INSERT INTO afk (start, end) VALUES (?,?)", (now, now + gap))
                    conn.commit()
                    pending_start = now + gap
                    continue
            cur_app = pick(rng)
            cur_title = rng.choice(TITLES[cur_app])
            conn.execute(
                "INSERT INTO events (start, end, app_id, title) VALUES (?,?,?,?)",
                (now, now + 1, cur_app, cur_title),
            )
            conn.commit()
            cur_id = conn.execute("SELECT last_insert_rowid()").fetchone()[0]
            rotate_at = now + rng.randint(3, 12) * 60
        else:
            conn.execute("UPDATE events SET end = ? WHERE id = ?", (now, cur_id))
            conn.commit()

        rpc.call(
            "settings.set",
            {"key": "demo.current_window", "value": f"{cur_app} — {cur_title}"},
        )
        time.sleep(args.interval)


if __name__ == "__main__":
    main()
