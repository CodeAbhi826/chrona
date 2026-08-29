#!/usr/bin/env bash
# Install + enable the Chrona KWin watcher for the current user (Plasma 6).
# The Chrona UI's Settings page runs the same steps; this script is for
# terminals and dotfiles.
set -euo pipefail

SRC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/chrona-watcher"
DEST="${HOME}/.local/share/kwin/scripts/chrona-watcher"

if [ ! -f "$SRC/metadata.json" ]; then
    echo "error: $SRC/metadata.json not found" >&2
    exit 1
fi

mkdir -p "$(dirname "$DEST")"
rm -rf "$DEST"
cp -r "$SRC" "$DEST"

kpackagetool6 --type=KWin/Script -u "$DEST" 2>/dev/null \
    || kpackagetool6 --type=KWin/Script -i "$DEST"
kwriteconfig6 --file kwinrc --group Plugins --key chrona-watcherEnabled true
dbus-send --session --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.start || true
dbus-send --session --dest=org.kde.KWin /KWin org.kde.KWin.reconfigure || true

echo "✓ Chrona KWin watcher installed and enabled."
echo "  Verify with:  qdbus6 org.chrona.Watcher /org/chrona/Watcher org.chrona.Watcher.Ping"
