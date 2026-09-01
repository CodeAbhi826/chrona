#!/usr/bin/env bash
# Install + enable the Chrona KWin watcher for the current user.
# Works on Plasma 6 (kpackagetool6) and Plasma 5 (kpackagetool5).
#
# The Chrona UI's Settings page runs the same logic; this script is for
# terminals, dotfiles and the universal install.sh.
#
# Usage: kwin/install.sh [SOURCE_DIR]
#   SOURCE_DIR defaults to the repo checkout next to this script. The
#   universal installer passes the /usr/share/chrona/kwin copy.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/chrona-watcher"
if [ "${1:-}" != "" ] && [ -f "${1}/metadata.json" ]; then
    SRC="$1"
fi
if [ ! -f "$SRC/metadata.json" ]; then
    echo "error: KWin script package not found at $SRC" >&2
    exit 1
fi

# kpackagetool wants the package SOURCE (a directory containing
# metadata.json). Pointing it at the installed copy breaks: --upgrade
# uninstalls (deletes) the installed package first and then fails with
# "No such file" because its own source disappeared. The script therefore
# never cp's anything itself — kpackagetool installs into
# ~/.local/share/kwin/scripts on its own.
KPT="$(command -v kpackagetool6 || command -v kpackagetool5 || true)"
KWC="$(command -v kwriteconfig6 || command -v kwriteconfig5 || true)"
if [ -z "$KPT" ]; then
    echo "error: kpackagetool not found — install your distro's KPackage tools" >&2
    echo "  arch:      pacman -S kpackage" >&2
    echo "  debian:    apt install kpackagetool6" >&2
    echo "  ubuntu:    apt install kf6-kpackage" >&2
    echo "  fedora:    dnf install kf6-kpackage" >&2
    echo "  opensuse:  zypper install kf6-kpackage" >&2
    exit 1
fi

if "$KPT" --type=KWin/Script --list 2>/dev/null | grep -q chrona-watcher; then
    "$KPT" --type=KWin/Script -u "$SRC"
else
    "$KPT" --type=KWin/Script -i "$SRC"
fi

if [ -n "$KWC" ]; then
    # The official KDE docs enable scripts via kwinrc [Plugins]; some
    # Plasma 5 installs used [Scripts]. Writing both keys is harmless
    # (the other subsystem ignores unknown keys) and covers everything.
    "$KWC" --file kwinrc --group Plugins --key chrona-watcherEnabled true || true
    "$KWC" --file kwinrc --group Scripts --key chrona-watcherEnabled true || true
fi

# Load enabled scripts now (normally done at KWin start) and re-read config.
dbus-send --session --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.start \
    >/dev/null 2>&1 || true
dbus-send --session --dest=org.kde.KWin /KWin org.kde.KWin.reconfigure \
    >/dev/null 2>&1 || true

# Verify the daemon side answered: the D-Bus intake owns org.chrona.Watcher.
if dbus-send --session --print-reply=literal \
        --dest=org.chrona.Watcher /org/chrona/Watcher org.chrona.Watcher.Ping \
        2>/dev/null | grep -q chrona; then
    echo "OK: Chrona KWin watcher installed and the daemon is reachable."
else
    echo "KWin watcher installed, but the daemon did not answer Ping."
    echo "  start it with:  systemctl --user enable --now chrona"
fi
