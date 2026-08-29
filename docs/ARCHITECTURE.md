# Chrona architecture

Chrona is three layers, each replaceable, glued by boring technology
(a Unix socket, one-line JSON, one SQLite file).

```
watchers (per compositor)  →  chronad (daemon)  →  chrona (UI)
```

## 1. Watcher layer — `crates/daemon/src/watchers/`

Watchers answer exactly one question: *which window is active right now?*
They push `Event::Window { app, title }` into the daemon's channel.

| Backend | File | Mechanism | Fires on |
|---|---|---|---|
| KDE Plasma Wayland | `kwin/chrona-watcher` (JS) + `dbus.rs` | KWin script `callDBus` → `org.chrona.Watcher.ActiveWindowChanged` | every `workspace.windowActivated` |
| wlroots Wayland | `watchers/wayland.rs` | `zwlr_foreign_toplevel_manager_v1` events | toplevel `state: activated` |
| X11 | `watchers/x11.rs` | `_NET_ACTIVE_WINDOW` + `WM_CLASS` polling (2 s) | focus change |

App identity is normalised to lowercase WM_CLASS / app_id ("firefox",
"org.telegram.desktop"). Window titles ride along for categorisation rules
and the per-app detail view.

**Why the KWin script exists:** KWin does not implement
`wlr-foreign-toplevel`. D-Bus is the one channel KWin scripts can always
reach, so a 30-line script pushes events instead. It's the same trust model
as the socket API: session-bus only, no privileges.

### Idle detection — `idle.rs`

AFK data is what makes totals honest. Providers, tried per session:

- **KDE / anything with a session bus**: poll `org.freedesktop.ScreenSaver.GetActive` (15 s).
- **wlroots Wayland**: `ext_idle-notify-v1` with a 60 s threshold (push, no polling).
- **X11**: MIT-SCREEN-SAVER `QueryInfo` → ms since last input (15 s poll, 60 s threshold).

## 2. Daemon — `chronad`

- **State machine** (`state.rs`): folds the event stream into rows.
  Same window → one row, extended by a 10 s flusher. Window switch → close
  previous row, open new. `IdleStart` → close row + open AFK session;
  `IdleEnd` → close AFK session and re-open the last known window (KDE does
  not re-fire focus after unlock). A crash loses at most one flush interval.
- **Storage** (`chrona-store`): one SQLite file, WAL mode, two connections
  (writer + API readers). Schema: `events`, `afk`, `rules`, `goals`,
  `settings`. Everything raw is kept forever; aggregation is done at query
  time so rule edits re-categorise history for free.
- **Stats** (`chrona-core`): pure functions — AFK subtraction (interval
  arithmetic, splits events), per-app/category/title aggregation, hourly
  buckets, day-splitting across midnight in local time, unlocks counting.
  Fully unit-tested, no I/O.

### The API

`$XDG_RUNTIME_DIR/chrona.sock` (mode `0600`), one JSON object per line:

```json
{"id": 7, "cmd": "day", "args": {"date": "2026-08-29"}}
{"id": 8, "ok": true, "data": { … }}
```

| Command | Args | Returns |
|---|---|---|
| `status` | — | version, uptime, watcher, idle provider, current window, db path, `paused` |
| `pause.set` | `paused` (bool) | suspends recording; paused time counts as AFK. Persists across restarts |
| `day` | `date?` | totals, `afk_seconds`, apps, categories, hourly[24], `timeline[]` (app segments), longest session, unlocks |
| `week` / `month` | `offset?` | same as `day` (+ `days[]`, + `prev_total_seconds` for week) |
| `range` | `from`,`to` | same as `day` + `days[]` |
| `app` | `app_id`, `days?` | top window titles, daily totals |
| `rules` / `rule.add` / `rule.del` | pattern, field, category, priority | rule list / new id |
| `goals` / `goal.set` / `goal.del` | kind (`app`\|`category`\|`total`), key, limit_seconds, enabled | goals + `used_seconds` today; `total` is the overall screen-time goal |
| `settings.get` / `settings.set` | key, value | persisted UI settings (theme, `notify`, …) |
| `export` | `from?`, `to?` | full JSON dump |
| `purge` | `before` | deletes older rows |

Try it:

```bash
echo '{"id":1,"cmd":"status"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock
```

## 3. UI — `chrona` (Slint)

- Declared in `crates/ui/ui/*.slint`; Rust glue only builds view-models and
  polls the daemon every 5 s (plus an immediate refresh flag for user
  actions).
- **Theme** is a global two-palette token set: light (default) and dark;
  flipping `Theme.dark` re-colours every widget including
  category colors.
- Charts are hand-drawn (rings/donuts are SVG arc `Path` commands computed in
  Rust; bars/heatmaps are plain rectangles) — no chart library, no webview.
- Desktop-first layout: top bar + unboxed nav rail, card grids, min 1080×700 — not a
  phone app stretched. HiDPI-safe (logical pixels everywhere).
- Fonts: Google Sans if installed, else bundled Inter (extracted to
  `~/.cache/chrona/Inter.ttf` and set via `SLINT_DEFAULT_FONT`).

## Performance notes

- Daemon: one thread per concern (watcher, idle, flush, API accept) + one
  thread per API connection. Rust std only on the hot path; zbus is confined
  to D-Bus intake.
- SQLite indexes on `events(start)` and `events(app_id)`; typical day
  queries are a few thousand rows — sub-millisecond.
- The UI binary is ~5 MB, the daemon ~4 MB (release, stripped, thin-LTO).
