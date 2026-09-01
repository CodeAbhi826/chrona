# Watcher setup guide

Chrona records window usage through one of three watcher backends, chosen
automatically from your session environment. This page covers setup,
verification and troubleshooting per compositor.

## How the choice is made (`watchers/mod.rs`)

| Environment | Watcher | Idle |
|---|---|---|
| `XDG_SESSION_TYPE=x11` | X11 EWMH poll | MIT-SCREEN-SAVER |
| Wayland + `KDE_FULL_SESSION` | KWin script → D-Bus | ScreenSaver D-Bus |
| Wayland + `SWAYSOCK` / `HYPRLAND_INSTANCE_SIGNATURE` / `NIRI_SOCKET` / `RIVER_UNIX_SOCKET` | wlr-foreign-toplevel | ext-idle-notify |
| anything else (e.g. GNOME Wayland) | none in v0.2 | ScreenSaver D-Bus |

Check what your daemon picked:

```bash
echo '{"id":1,"cmd":"status"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock
```

The `watcher` and `idle_provider` fields name the active backends.

---

## KDE Plasma 6 — Wayland (recommended path)

KWin does not implement `wlr-foreign-toplevel`, so Chrona ships a KWin
script that reports window activations over the session bus.

**Install (either):**

- Chrona → Settings → *Install / update KWin watcher*, or
- `./kwin/install.sh` from a checkout (or `bash install.sh` from the repo
  root — the universal installer runs it on KDE), or
- the package already placed it at `/usr/share/chrona/kwin/chrona-watcher`.

The tooling always points kpackagetool at the package **source** (checkout or
`/usr/share` copy), never at the installed copy in `~/.local/share/kwin/scripts`
— `--upgrade` deletes the installed package first, so upgrading "from" it
would fail with `No such file`.

**Verify:**

```bash
# 1. the daemon owns its D-Bus name
qdbus6 org.chrona.Watcher /org/chrona/Watcher org.chrona.Watcher.Ping
# → chrona

# 2. the script is known to KWin
kpackagetool6 --type=KWin/Script --list | grep chrona

# 3. focus a window, then check the daemon noticed
echo '{"id":2,"cmd":"status"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock
# → "current_window": { "app_id": "...", "title": "..." }
```

If nothing arrives: log out/in (KWin loads enabled scripts at startup), and
check `journalctl --user -u chrona` for `D-Bus intake unavailable`.

**Notes**

- Idle/lock detection polls `org.freedesktop.ScreenSaver.GetActive`, which
  Plasma implements — screen-lock time is subtracted automatically.
- After unlocking, the last active window is re-opened as the active one
  (KDE does not emit a focus event on unlock).

## KDE Plasma — X11 session

No script needed: the X11 watcher (`_NET_ACTIVE_WINDOW`) and the
MIT-SCREEN-SAVER idle provider start on their own.

## Sway / Hyprland / river / niri (wlroots protocol family)

Works out of the box. Requirements:

- compositor exposes `zwlr_foreign_toplevel_manager_v1` (all of the above do),
- for idle: `ext-idle-notify-v1` (Sway ≥ 1.8, Hyprland, river, niri do).

Ensure the daemon is started *inside* the session (systemd user service
inherited environment is fine; `exec systemctl --user start chrona` from
your Sway config also works) so `SWAYSOCK`-style env vars are visible.

**Caveat:** some compositors set app_id per-app inconsistently
(XWayland windows may report their WM_CLASS). Chrona normalises to lowercase
and treats XWayland classes like any other app id.

## Generic X11 (i3, XFCE, anything on Xorg)

Works out of the box via EWMH polling (2 s resolution) +
MIT-SCREEN-SAVER idle (60 s threshold). Tiling window managers without an
EWMH-compliant active-window hint are rare; if `status` shows an X11 watcher
but `current_window` stays `null`, file an issue with your WM's name.

## GNOME Wayland

v0.2 tracks idle/AFK only — GNOME Shell (like KWin) does not expose
toplevels to normal clients. A small GNOME Shell extension pushing to the
same `org.chrona.Watcher` D-Bus interface is planned; the daemon side is
already listening for it, so the extension will be the only new piece.

## PWAs and browser windows

PWAs (Chrome/Chromium `--app` mode, Edge, Firefox SSB) create real toplevel
windows with their own WM_CLASS and title, so Chrona tracks them as separate
apps automatically — no extension needed for app-level granularity.
Per-URL granularity inside normal browser windows arrives with the planned
browser companion; today, title rules (e.g. Netflix/YouTube) already
categorise streaming time correctly.

## Troubleshooting

```bash
# daemon logs
journalctl --user -u chrona -f

# is the socket alive?
ls -l $XDG_RUNTIME_DIR/chrona.sock   # mode should be 0600

# raw query
echo '{"id":1,"cmd":"status"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock

# today after a few minutes of usage
echo '{"id":1,"cmd":"day"}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock | jq .data.total_seconds
```

| Symptom | Fix |
|---|---|
| `Daemon offline` in the UI | `systemctl --user enable --now chrona` |
| watcher = `KDE … script` but no windows | run the installer again; log out/in |
| watcher = `no wlr-foreign-toplevel` | you're on a compositor without the protocol (GNOME?); see above |
| totals include lock-screen time | idle provider missing — check `idle_provider` in `status` |
| app names look wrong (e.g. `org.foo.bar`) | that's the app_id; add a rename/categorisation rule |
