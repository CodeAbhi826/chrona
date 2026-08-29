# Chrona demo harness

Run the **real Chrona daemon** with a **simulated watcher feed** and browse
the dashboard in any browser — no compositor, no Plasma, no Linux desktop
required. Handy for trying Chrona before installing it, for screenshots,
and for developing the UI.

```bash
./tools/demo/run_demo.sh            # → http://localhost:3000
PORT=8080 ./tools/demo/run_demo.sh  # custom port
```

## What is real vs simulated

| Piece | Status |
|---|---|
| `chronad` (storage, rule engine, stats, goals, settings API) | **real** — every number is computed by the Rust daemon from real SQLite rows |
| Dashboard web UI | faithful mirror of the native Slint app (same tokens, same formatting) served by `server.py` |
| Window events | **simulated** — `seed.py` writes 50 days of realistic usage, `ticker.py` keeps one "current window" event open so the dashboard is live |
| AFK / unlocks | simulated AFK sessions (what `chronad`'s idle watchers produce on a real desktop) |

## Pieces

- `seed.py` — generates ~50 days of an Arch + KDE persona (VS Code, Konsole,
  Firefox incl. Netflix titles, Steam, Discord, Spotify, Krita, Figma, Brave
  PWAs) directly into `chrona.db`, using the exact row shape the watcher
  pipeline produces. Categorisation happens in real Chrona rules at query
  time (e.g. *Netflix in Firefox* lands in **Media & Streaming**, not Browsers).
- `ticker.py` — keeps the dashboard alive: rotates the "current window"
  every few minutes, inserts occasional AFK gaps, seeds demo goals through
  the real socket API, and publishes `demo.current_window` for the
  "Right now" line.
- `server.py` — HTTP bridge onto chronad's Unix-socket JSON API
  (read-only `GET /api/*`, plus a small write whitelist for goals/rules/
  settings so the dashboard is fully interactive).
- `web/` — the dashboard itself. One HTML/CSS/JS page, zero dependencies,
  Inter (bundled, OFL) with Google Sans as first local fallback.

## Notes

- The demo database lives in `/tmp/chrona-demo` (override with
  `CHRONA_RUN_DIR`). Delete it to re-seed from scratch.
- Everything still runs 100% locally — the server binds wherever you point
  it and only talks to the daemon socket.
