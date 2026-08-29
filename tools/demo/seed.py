#!/usr/bin/env python3
"""Chrona demo data seeder.

Fills a chrona.db with `--days` days of realistic Linux-desktop usage
(Arch + KDE persona: VS Code, Konsole, Firefox, Steam, Discord, Spotify,
Figma/Krita, PWAs via Brave) so the dashboard can be explored without a
compositor. Writes the exact same rows chronad's watcher pipeline would
produce, so all aggregation / categorisation happens in real Chrona code.

Stop chronad while seeding (it reads SQLite in WAL mode, so a running
daemon is tolerable, but a stopped one is cleaner).

Usage:
    python3 tools/demo/seed.py --db /tmp/chrona-demo/data/chrona.db [--days 50]
"""

import argparse
import datetime as dt
import random
import sqlite3
import time

TITLES = {
    "firefox": [
        "Arch Wiki — pacman",
        "GitHub · chrona-linux/chrona",
        "Hacker News",
        "r/linux — Reddit",
        "Gmail — Inbox (3)",
        "MDN — setInterval",
        "Phoronix — Linux Hardware Reviews",
        "Rust Book — Ch. 15 Smart Pointers",
    ],
    "firefox:netflix": [
        "Netflix — Watch TV Shows Online",
        "Netflix — Breaking Bad",
    ],
    "code": [
        "main.rs — chronad — Visual Studio Code",
        "stats.rs — chrona-core — Visual Studio Code",
        "client.rs — chrona — Visual Studio Code",
        "app.slint — chrona — Visual Studio Code",
    ],
    "konsole": [
        "zsh — ~/projects/chrona",
        "yay -S chrona",
        "cargo test --workspace",
        "htop",
        "git — rebase -i main",
    ],
    "kate": ["chrona-kwin.js — Kate", "PKGBUILD — Kate", "todo.md — Kate"],
    "libreoffice-writer": ["quarterly-report.odt — LibreOffice Writer", "chrona-notes.odt — LibreOffice Writer"],
    "dolphin": ["Home — Dolphin", "projects — Dolphin"],
    "discord": ["#chrona-dev — Discord", "Linux Memes — Discord", "KDE Plasma — Discord"],
    "telegram-desktop": ["Arch Linux Group", "KDE Updates", "Rust Community"],
    "spotify": ["Lofi Beats — Spotify", "Deep Focus — Spotify", "Discover Weekly — Spotify"],
    "steam": ["Library — Steam", "Counter-Strike 2 — Steam", "Store — Steam"],
    "obs": ["OBS Studio 30.2 — Chrona demo", "OBS Studio — Recording"],
    "krita": ["chrona-hero.png — Krita", "mockup-dashboard.png — Krita"],
    "figma": ["Chrona Dashboard — Figma", "Icon set — Figma"],
    # PWAs installed through Brave — app_id is the browser, the *title*
    # rules still categorise them (e.g. "YouTube Music" -> Media).
    "brave": ["YouTube Music", "Notion — Tasks", "Figma — Files"],
    "systemsettings": ["System Settings — Colors & Themes", "System Settings — Displays"],
}

# Sequential day plans: (app, min_m, max_m, probability). "LUNCH"/"DINNER"
# become AFK sessions (which is what produces "unlocks").
WEEKDAY = [
    ("firefox", 12, 35, 1.0),
    ("telegram-desktop", 6, 18, 1.0),
    ("code", 75, 135, 1.0),
    ("konsole", 18, 42, 0.9),
    ("spotify", 35, 80, 0.55),
    ("code", 45, 90, 0.8),
    ("firefox", 15, 40, 0.9),
    ("discord", 12, 35, 0.8),
    ("LUNCH", 40, 65, 1.0),
    ("code", 60, 120, 1.0),
    ("kate", 12, 30, 0.6),
    ("figma", 25, 60, 0.4),
    ("krita", 25, 65, 0.3),
    ("konsole", 12, 35, 0.7),
    ("firefox", 10, 30, 0.8),
    ("DINNER", 35, 60, 1.0),
    ("steam", 60, 150, 0.45),
    ("firefox:netflix", 55, 110, 0.35),
    ("obs", 35, 80, 0.18),
    ("discord", 18, 45, 0.7),
    ("brave", 20, 55, 0.5),
    ("spotify", 25, 60, 0.5),
    ("telegram-desktop", 8, 20, 0.8),
]

WEEKEND = [
    ("firefox", 20, 55, 1.0),
    ("telegram-desktop", 10, 25, 1.0),
    ("dolphin", 6, 15, 0.6),
    ("steam", 110, 220, 0.85),
    ("discord", 35, 80, 0.8),
    ("firefox:netflix", 80, 150, 0.55),
    ("LUNCH", 45, 70, 1.0),
    ("steam", 60, 140, 0.5),
    ("spotify", 40, 85, 0.7),
    ("code", 30, 75, 0.35),
    ("konsole", 15, 40, 0.3),
    ("krita", 30, 70, 0.25),
    ("systemsettings", 8, 20, 0.5),
    ("DINNER", 35, 55, 1.0),
    ("brave", 30, 60, 0.6),
    ("firefox", 30, 70, 0.7),
    ("discord", 20, 50, 0.6),
    ("telegram-desktop", 8, 18, 0.7),
]

DDL = """
CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL,
    app_id TEXT NOT NULL,
    title TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_start ON events(start);
CREATE INDEX IF NOT EXISTS idx_events_app ON events(app_id);
CREATE TABLE IF NOT EXISTS afk (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_afk_start ON afk(start);
CREATE TABLE IF NOT EXISTS rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    pattern TEXT NOT NULL,
    field TEXT NOT NULL,
    category TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100
);
CREATE TABLE IF NOT EXISTS goals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    limit_seconds INTEGER NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    UNIQUE(kind, key)
);
CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"""


def day_events(day: dt.date, now_ts: int):
    """Yield (start, end, app_id, title) and (start, end) AFK tuples for one day."""
    rng = random.Random(f"chrona-{day.isoformat()}")
    weekend = day.weekday() >= 5
    plan = WEEKEND if weekend else WEEKDAY

    start_h = (10.25 if weekend else 9.0) + rng.random() * (1.5 if weekend else 1.2)
    t = time.mktime(dt.datetime.combine(day, dt.time(0, 0)).timetuple()) + start_h * 3600
    t = int(t)
    now = int(time.time()) if day == dt.date.today() else None
    # Early-morning guarantee: if "today" would start less than an hour
    # before now, pretend the day began a few hours ago so the dashboard
    # always has something to show.
    if now is not None and t > now - 3600:
        t = now - rng.randint(150, 210) * 60

    events, afk = [], []
    for app, lo, hi, prob in plan:
        if rng.random() > prob:
            continue
        dur = rng.randint(lo, hi) * 60
        if app in ("LUNCH", "DINNER"):
            afk.append((t, t + dur))
            t += dur
            continue
        if now is not None and t >= now:
            break
        end = t + dur
        if now is not None:
            end = min(end, now)
        # Long blocks become 2 events with different titles (more realistic
        # window switching, richer "Most time in" lists).
        if dur > 60 * 60 and len(TITLES[app]) > 1 and end - t == dur:
            mid = t + dur // 2
            events.append((t, mid, app, rng.choice(TITLES[app])))
            events.append((mid, end, app, rng.choice(TITLES[app])))
        elif end > t:
            events.append((t, end, app, rng.choice(TITLES[app])))
        t = end
        # small break between blocks; >= 8 min counts as AFK (an "unlock")
        gap = rng.randint(2, 12) * 60
        if gap >= 8 * 60:
            afk.append((t, t + gap))
        t += gap
    return events, afk


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--days", type=int, default=50)
    ap.add_argument("--fresh", action="store_true", help="wipe events/afk first")
    args = ap.parse_args()

    today = dt.date.today()
    # Guarantee "today" has visible data even if the clock says 8am.
    now_ts = int(time.time())

    conn = sqlite3.connect(args.db, timeout=10)
    conn.execute("PRAGMA journal_mode=WAL")
    conn.executescript(DDL)
    if args.fresh:
        conn.execute("DELETE FROM events")
        conn.execute("DELETE FROM afk")
    total_ev = total_afk = 0
    for i in range(args.days - 1, -1, -1):
        day = today - dt.timedelta(days=i)
        events, afk = day_events(day, now_ts)
        conn.executemany(
            "INSERT INTO events (start, end, app_id, title) VALUES (?,?,?,?)", events
        )
        conn.executemany("INSERT INTO afk (start, end) VALUES (?,?)", afk)
        total_ev += len(events)
        total_afk += len(afk)
    conn.commit()

    cur = conn.execute(
        "SELECT COUNT(*), COALESCE(SUM(end-start),0) FROM events "
        "WHERE start >= ?",
        (time.mktime(dt.datetime.combine(today, dt.time(0, 0)).timetuple()),),
    )
    n_today, secs_today = cur.fetchone()
    hours = secs_today / 3600
    print(
        f"seeded {args.days} days: {total_ev} events, {total_afk} afk sessions | "
        f"today so far: {n_today} events, {hours:.1f}h"
    )
    conn.close()


if __name__ == "__main__":
    main()
