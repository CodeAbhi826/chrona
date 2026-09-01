#!/usr/bin/env bash
# Chrona — universal installer.
#
# One command installs Chrona on any mainstream glibc x86_64 Linux desktop:
# binaries from the GitHub release, the systemd user unit, the app-menu
# entry + icons, and (on KDE Plasma) the KWin watcher script. Everything is
# detected automatically: distro, package manager, privileges, downloader,
# desktop environment and session type.
#
# Tested combinations:
#   distros : Arch / Manjaro / EndeavourOS (pacman) · Debian / Ubuntu / Mint /
#             Pop!_OS (apt) · Fedora / RHEL / Rocky / Alma (dnf, yum) ·
#             openSUSE (zypper) · Void (xbps) · anything else (no root
#             needed for the core install)
#   shells  : bash, zsh, fish, anything — a system install needs no shell
#             config; --user mode edits the rc file of the login shell
#   desktops: KDE Plasma 6/5 (Wayland or X11) · Sway/Hyprland/river/niri ·
#             any X11 session · GNOME (idle/AFK tracking; window events
#             need the planned Shell extension)
#
# Usage:
#   bash install.sh                # system install (uses sudo), latest release
#   bash install.sh v0.2.2         # pin a specific release
#   bash install.sh --user         # no-root install into ~/.local
#
# From a repo checkout:  bash install.sh
# From anywhere:         curl -fsSL https://raw.githubusercontent.com/CodeAbhi826/chrona/main/install.sh | bash
set -euo pipefail

REPO="CodeAbhi826/chrona"

# ---------------------------------------------------------------- helpers ----
if [ -t 1 ]; then
    BOLD=$'\033[1m'; CYAN=$'\033[1;36m'
    GREEN=$'\033[1;32m'; YELLOW=$'\033[1;33m'; RED=$'\033[1;31m'; RST=$'\033[0m'
else
    BOLD=""; CYAN=""; GREEN=""; YELLOW=""; RED=""; RST=""
fi
log()  { printf '%s==> %s%s\n' "$CYAN" "$*" "$RST"; }
ok()   { printf '%s -> %s%s\n' "$GREEN" "$*" "$RST"; }
warn() { printf '%s -> %s%s\n' "$YELLOW" "$*" "$RST"; }
die()  { printf '%serror: %s%s\n' "$RED" "$*" "$RST" >&2; exit 1; }

# ------------------------------------------------------------------ options ---
MODE=system
TAG=""
for arg in "$@"; do
    case "$arg" in
        --user) MODE=user ;;
        --system) MODE=system ;;
        v[0-9]*) TAG="$arg" ;;
        [0-9]*.[0-9]*.[0-9]*) TAG="v$arg" ;;
        -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
        *) die "unknown option: $arg (try --help)" ;;
    esac
done

# ------------------------------------------------------------ sanity checks ---
[ "$(uname -s)" = "Linux" ] || die "this installer is for Linux"
case "$(uname -m)" in
    x86_64) ;;
    aarch64|arm64) die "prebuilt releases are x86_64-only for now — build from source (README, Install)" ;;
    *) die "unsupported architecture: $(uname -m) — build from source" ;;
esac
if ldd --version 2>&1 | head -1 | grep -qi musl; then
    die "musl libc detected — the prebuilt binaries need glibc; build from source instead (README, Install)"
fi

# downloader: curl → wget → gh
DL=""
command -v curl >/dev/null && DL=curl
[ -z "$DL" ] && command -v wget >/dev/null && DL=wget
[ -z "$DL" ] && command -v gh >/dev/null && DL=gh
[ -n "$DL" ] || die "need curl, wget or the GitHub CLI to download the release"

fetch() { # $1 url, $2 dest file
    case "$DL" in
        curl) curl -fsSL --retry 3 --connect-timeout 15 -o "$2" "$1" ;;
        wget) wget -q --tries=3 -O "$2" "$1" ;;
    esac
}
url_up() { # $1 url → exit 0 if downloadable
    case "$DL" in
        curl) [ "$(curl -sIL -o /dev/null -w '%{http_code}' --connect-timeout 15 "$1")" = "200" ] ;;
        wget) wget -q --spider "$1" ;;
        gh)   [ -n "$GH_TAG" ] && gh release view "$GH_TAG" -R "$REPO" >/dev/null 2>&1 ;;
    esac
}

# ------------------------------------------------------------------ distro ----
if [ -r /etc/os-release ]; then . /etc/os-release; fi
ID="${ID:-linux}"; ID_LIKE="${ID_LIKE:-}"
DISTRO="other"; PM=""
case " $ID $ID_LIKE " in
    *" arch "*|*"arch"*|*"manjaro"*|*"endeavouros"*|*"garuda"*|*"artix"*|*"cachyos"*) DISTRO=arch; PM=pacman ;;
    *" debian "*|*"ubuntu"*|*"mint"*|*"pop"*|*"elementary"*|*"zorin"*|*"deepin"*|*"kali"*) DISTRO=debian; PM=apt ;;
    *" fedora "*|*"rhel"*|*"centos"*|*"rocky"*|*"alma"*) DISTRO=fedora; PM=dnf ;;
    *"opensuse"*|*" sles "*) DISTRO=opensuse; PM=zypper ;;
    *" void "*) DISTRO=void; PM=xbps ;;
    *"alpine"*) die "Alpine uses musl — build from source instead (README, Install)" ;;
    *"gentoo"*) DISTRO=gentoo; PM=emerge ;;
esac
if [ "$DISTRO" = "fedora" ] && ! command -v dnf >/dev/null; then PM=yum; fi

SUDO=""
if [ "$(id -u)" -eq 0 ]; then
    :
elif [ "$MODE" = "user" ]; then
    :
elif command -v sudo >/dev/null; then
    SUDO="sudo"
elif command -v doas >/dev/null; then
    SUDO="doas"
else
    warn "no sudo/doas found — falling back to a --user install"
    MODE=user
fi
as_root() { # run a command with root rights when in system mode
    if [ -n "$SUDO" ]; then $SUDO "$@"; else "$@"; fi
}

pm_install() { # best-effort package install (never fatal)
    [ -n "$PM" ] || { warn "unknown distro — install these manually if missing: $*"; return 0; }
    log "installing packages with $PM: $*"
    case "$PM" in
        pacman) as_root pacman -S --needed --noconfirm "$@" || true ;;
        apt)    as_root sh -c 'apt-get update -qq && exec apt-get install -y "$@"' _ "$@" || true ;;
        dnf)    as_root dnf install -y "$@" || true ;;
        yum)    as_root yum install -y "$@" || true ;;
        zypper) as_root zypper --non-interactive install "$@" || true ;;
        xbps)   as_root xbps-install -Sy "$@" || true ;;
        emerge) as_root emerge --oneshot "$@" || true ;;
    esac
}

# ------------------------------------------------- repo checkout (optional) ---
# When run from a checkout, reuse its packaging/kwin assets; when piped from
# the network, everything needed is downloaded or embedded below.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" 2>/dev/null && pwd || true)"
REPO_DIR=""
if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ] && [ -d "$SCRIPT_DIR/kwin" ]; then
    REPO_DIR="$SCRIPT_DIR"
fi
raw() { echo "https://raw.githubusercontent.com/$REPO/main/$1"; }

# --------------------------------------------------------------- release ------
GH_TAG=""
if [ -z "$TAG" ]; then
    log "finding the latest release"
    [ "$DL" = gh ] && { TAG="$(gh release view -R "$REPO" --json tagName -q .tagName 2>/dev/null || true)"; }
    if [ -z "$TAG" ] && [ "$DL" = curl ]; then
        TAG="$(curl -sIL -o /dev/null -w '%{url_effective}' \
            "https://github.com/$REPO/releases/latest" | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' || true)"
    fi
    [ -n "$TAG" ] || die "cannot auto-detect the latest release — pass a version: bash install.sh vX.Y.Z"
fi
VER="${TAG#v}"
GH_TAG="$TAG"
ASSET="chrona-$TAG-x86_64-linux-gnu.tar.gz"
URL="https://github.com/$REPO/releases/download/$TAG/$ASSET"

TMP="$(mktemp -d)"
trap 'command rm -rf -- "$TMP"' EXIT   # plain `rm`, immune to interactive aliases

# If the release was tagged seconds ago, CI may still be building it. Wait.
tries=0
until url_up "$URL"; do
    tries=$((tries + 1))
    if [ "$tries" -eq 1 ]; then
        log "release $TAG is still being built by CI — waiting for it to publish"
        printf '    (this normally takes 5–8 minutes after pushing a tag)\n'
    fi
    [ "$tries" -gt 45 ] && die "release $TAG never appeared (CI failed? check: https://github.com/$REPO/actions)"
    sleep 20
done

log "downloading chrona $VER"
case "$DL" in
    gh) gh release download "$TAG" -R "$REPO" -p "$ASSET" -D "$TMP" ;;
    *)  fetch "$URL" "$TMP/$ASSET" ;;
esac
[ -f "$TMP/$ASSET" ] || die "download failed"
tar -C "$TMP" -xzf "$TMP/$ASSET"
[ -x "$TMP/chronad" ] && [ -x "$TMP/chrona" ] || die "release tarball is missing the binaries"

# --------------------------------------------------------- runtime libraries ---
missing_libs() { ldd "$1" 2>/dev/null | grep 'not found' | awk '{print $1}' | sort -u; }
MISSING="$(missing_libs "$TMP/chrona"; missing_libs "$TMP/chronad"; true)"
if [ -n "$MISSING" ]; then
    log "installing missing runtime libraries ($MISSING)"
    case "$DISTRO" in
        arch)    pm_install libxkbcommon libxkbcommon-x11 fontconfig freetype2 wayland libx11 libxcb ;;
        debian)  pm_install libxkbcommon0 libxkbcommon-x11-0 libfontconfig1 libfreetype6 libwayland-client0 libx11-6 libxcb1 ;;
        fedora)  pm_install libxkbcommon libxkbcommon-x11 fontconfig freetype libwayland-client libX11 libxcb ;;
        opensuse) pm_install libxkbcommon0 libxkbcommon-x11-0 libfontconfig1 libfreetype6 libwayland-client0 libX11-6 libxcb1 ;;
        void)    pm_install libxkbcommon libxkbcommon-x11 fontconfig freetype wayland libX11 libxcb ;;
        *)       warn "install these libraries with your package manager: $MISSING" ;;
    esac
    MISSING="$(missing_libs "$TMP/chrona"; missing_libs "$TMP/chronad"; true)"
    [ -z "$MISSING" ] || die "still missing after install: $MISSING — install them and re-run"
fi

# ----------------------------------------------------------------- binaries ----
if [ "$MODE" = system ]; then
    PREFIX="${CHRONA_PREFIX:-/usr}"
    BIN="$PREFIX/bin"
    log "installing binaries to $BIN"
    as_root install -Dm755 "$TMP/chronad" "$BIN/chronad"
    as_root install -Dm755 "$TMP/chrona"  "$BIN/chrona"
else
    PREFIX="$HOME/.local"
    BIN="$PREFIX/bin"
    mkdir -p "$BIN"
    log "installing binaries to $BIN (user mode)"
    install -Dm755 "$TMP/chronad" "$BIN/chronad"
    install -Dm755 "$TMP/chrona"  "$BIN/chrona"
fi
ok "chronad + chrona $VER installed"

# ---------------------------------------------------------------- unit file ----
unit_src=""
[ -n "$REPO_DIR" ] && [ -f "$REPO_DIR/packaging/chrona.service" ] && unit_src="$REPO_DIR/packaging/chrona.service"
if [ -z "$unit_src" ]; then
    unit_src="$TMP/chrona.service"
    cat > "$unit_src" <<EOF
[Unit]
Description=Chrona - digital wellbeing daemon
Documentation=https://github.com/CodeAbhi826/chrona
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=$BIN/chronad
Restart=on-failure
RestartSec=3

[Install]
WantedBy=default.target
EOF
elif [ "$BIN" != "/usr/bin" ]; then
    sed "s|^ExecStart=.*|ExecStart=$BIN/chronad|" "$unit_src" > "$TMP/chrona.service"
    unit_src="$TMP/chrona.service"
fi

AUTOSTART=""
if [ "$MODE" = system ]; then
    UNIT_DIR="/usr/lib/systemd/user"
    [ -d /usr/lib/systemd/user ] || UNIT_DIR="/lib/systemd/user"
    log "installing the systemd user unit to $UNIT_DIR"
    as_root install -Dm644 "$unit_src" "$UNIT_DIR/chrona.service"
else
    UNIT_DIR="$HOME/.config/systemd/user"
    mkdir -p "$UNIT_DIR"
    sed "s|^ExecStart=.*|ExecStart=$BIN/chronad|" "$unit_src" > "$UNIT_DIR/chrona.service"
fi

if command -v systemctl >/dev/null 2>&1 \
        && (timeout 10 systemctl --user show-environment >/dev/null 2>&1 \
            || command -v systemd-userdbd >/dev/null); then
    log "enabling + starting the daemon (systemd user unit)"
    systemctl --user daemon-reload
    systemctl --user enable chrona >/dev/null 2>&1 || true
    systemctl --user restart chrona \
        || warn "systemctl --user restart chrona failed — check: systemctl --user status chrona"
else
    # No systemd user session (rare): fall back to XDG autostart + start now.
    warn "systemd user session unavailable — using ~/.config/autostart instead"
    AUTOSTART=1
    mkdir -p "$HOME/.config/autostart"
    cat > "$HOME/.config/autostart/chronad.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Chrona daemon
Exec=$BIN/chronad
X-GNOME-Autostart-enabled=true
EOF
    (setsid "$BIN/chronad" >/dev/null 2>&1 &)
fi

# ------------------------------------------------------------ menu + icons ----
desktop_src=""
[ -n "$REPO_DIR" ] && [ -f "$REPO_DIR/packaging/chrona.desktop" ] && desktop_src="$REPO_DIR/packaging/chrona.desktop"
if [ -z "$desktop_src" ]; then
    desktop_src="$TMP/chrona.desktop"
    cat > "$desktop_src" <<'EOF'
[Desktop Entry]
Type=Application
Name=Chrona
GenericName=Digital Wellbeing
Comment=Screen time, per-app usage and daily goals — 100% local
Exec=chrona
Icon=chrona
Terminal=false
Categories=Utility;System;Monitor;
Keywords=screen time;wellbeing;usage;tracking;focus;
StartupWMClass=chrona
EOF
fi
if [ "$MODE" = user ] && [ "$BIN" != "/usr/bin" ]; then
    sed "s|^Exec=chrona$|Exec=$BIN/chrona|" "$desktop_src" > "$TMP/chrona.desktop"
    desktop_src="$TMP/chrona.desktop"
fi

icon_src() { # $1 repo-relative file, $2 fallback temp name; prints path or fails
    if [ -n "$REPO_DIR" ] && [ -f "$REPO_DIR/$1" ]; then
        echo "$REPO_DIR/$1"
        return 0
    fi
    local t="$TMP/$2"
    if fetch "$(raw "$1")" "$t" 2>/dev/null; then
        echo "$t"
        return 0
    fi
    return 1
}
if [ "$MODE" = system ]; then
    APPS="/usr/share/applications"; HICOLOR="/usr/share/icons/hicolor"
    log "installing the app-menu entry and icons"
    as_root install -Dm644 "$desktop_src" "$APPS/chrona.desktop"
    svg="$(icon_src assets/icon.svg chrona.svg)"  && as_root install -Dm644 "$svg" "$HICOLOR/scalable/apps/chrona.svg" || true
    png="$(icon_src assets/icon-256.png chrona.png)" && as_root install -Dm644 "$png" "$HICOLOR/256x256/apps/chrona.png" || true
    command -v update-desktop-database >/dev/null && as_root update-desktop-database "$APPS" >/dev/null 2>&1 || true
else
    APPS="$HOME/.local/share/applications"; HICOLOR="$HOME/.local/share/icons/hicolor"
    log "installing the app-menu entry and icons (user mode)"
    mkdir -p "$APPS" "$HICOLOR/scalable/apps" "$HICOLOR/256x256/apps"
    install -Dm644 "$desktop_src" "$APPS/chrona.desktop"
    svg="$(icon_src assets/icon.svg chrona.svg)"  && install -Dm644 "$svg" "$HICOLOR/scalable/apps/chrona.svg" || true
    png="$(icon_src assets/icon-256.png chrona.png)" && install -Dm644 "$png" "$HICOLOR/256x256/apps/chrona.png" || true
    command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" >/dev/null 2>&1 || true
fi

# -------------------------------------------------------------- KWin script ----
DE="$(printf '%s %s' "${XDG_CURRENT_DESKTOP:-}" "${XDG_SESSION_DESKTOP:-}" | tr '[:upper:]' '[:lower:]')"
SESSION="${XDG_SESSION_TYPE:-}"
IS_KDE=false
case "$DE" in *kde*|*plasma*) IS_KDE=true ;; esac
if ! $IS_KDE && command -v kwin_wayland >/dev/null 2>&1; then IS_KDE=true; fi
if ! $IS_KDE && command -v kwin_x11 >/dev/null 2>&1; then IS_KDE=true; fi

KWIN_PKG_DONE=false
if $IS_KDE; then
    log "KDE Plasma detected — installing the KWin watcher script"
    if ! command -v kpackagetool6 >/dev/null && ! command -v kpackagetool5 >/dev/null; then
        case "$DISTRO" in
            arch)     pm_install kpackage kconfig ;;
            debian)   pm_install kpackagetool6 ;;
            fedora)   pm_install kf6-kpackage kf6-kconfig ;;
            opensuse) pm_install kf6-kpackage kf6-kconfig ;;
            *)        warn "kpackagetool not found — install Plasma's kpackage tools, then re-run" ;;
        esac
    fi
    if command -v kpackagetool6 >/dev/null || command -v kpackagetool5 >/dev/null; then
        ksrc=""
        if [ -n "$REPO_DIR" ] && [ -f "$REPO_DIR/kwin/chrona-watcher/metadata.json" ]; then
            ksrc="$REPO_DIR/kwin/chrona-watcher"
        else
            ksrc="$TMP/kwin-pkg"
            mkdir -p "$ksrc/contents/code"
            fetch "$(raw kwin/chrona-watcher/metadata.json)" "$ksrc/metadata.json" \
                && fetch "$(raw kwin/chrona-watcher/contents/code/main.js)" "$ksrc/contents/code/main.js" \
                || ksrc=""
        fi
        if [ -n "$ksrc" ]; then
            # Keep a stable system copy as the kpackagetool SOURCE (never the
            # installed copy — upgrade deletes that).
            if [ "$MODE" = system ]; then
                as_root mkdir -p /usr/share/chrona/kwin
                as_root cp -r "$ksrc" /usr/share/chrona/kwin/chrona-watcher
                ksrc=/usr/share/chrona/kwin/chrona-watcher
            else
                mkdir -p "$HOME/.local/share/chrona/kwin"
                cp -r "$ksrc" "$HOME/.local/share/chrona/kwin/chrona-watcher"
                ksrc="$HOME/.local/share/chrona/kwin/chrona-watcher"
            fi
            KPT="$(command -v kpackagetool6 || command -v kpackagetool5)"
            if "$KPT" --type=KWin/Script --list 2>/dev/null | grep -q chrona-watcher; then
                "$KPT" --type=KWin/Script -u "$ksrc" \
                    || "$KPT" --type=KWin/Script -i "$ksrc"
            else
                "$KPT" --type=KWin/Script -i "$ksrc"
            fi
            KWC="$(command -v kwriteconfig6 || command -v kwriteconfig5 || true)"
            if [ -n "$KWC" ]; then
                # [Plugins] per the KDE docs; [Scripts] for older Plasma 5 layouts.
                "$KWC" --file kwinrc --group Plugins --key chrona-watcherEnabled true || true
                "$KWC" --file kwinrc --group Scripts --key chrona-watcherEnabled true || true
            fi
            dbus-send --session --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.start >/dev/null 2>&1 || true
            dbus-send --session --dest=org.kde.KWin /KWin org.kde.KWin.reconfigure >/dev/null 2>&1 || true
            KWIN_PKG_DONE=true
            ok "KWin watcher installed + enabled"
        else
            warn "could not fetch the KWin script — run Chrona → Settings → Install KWin watcher later"
        fi
    fi
else
    case "$DE $SESSION" in
        *gnome*) warn "GNOME: idle/AFK tracking works; per-app window events need the GNOME Shell extension (roadmap)" ;;
        *sway*|*hypr*|*river*|*niri*) ok "wlroots compositor — wlr-foreign-toplevel works out of the box" ;;
        *) [ "$SESSION" = x11 ] && ok "X11 session — EWMH watcher works out of the box" || warn "unrecognised desktop ($DE) — the daemon will try ScreenSaver idle polling" ;;
    esac
fi

# --------------------------------------------------------- PATH for --user ----
if [ "$MODE" = user ] && [ "$BIN" = "$HOME/.local/bin" ]; then
    case ":$PATH:" in
        *":$HOME/.local/bin:"*) ;;
        *)
            log "adding ~/.local/bin to PATH"
            shell="$(basename "${SHELL:-/bin/bash}")"
            case "$shell" in
                fish)
                    f="$HOME/.config/fish/config.fish"; mkdir -p "$(dirname "$f")"
                    grep -q 'fish_add_path.*\.local/bin' "$f" 2>/dev/null || \
                        printf 'if status --is-interactive\n    fish_add_path -U $HOME/.local/bin\nend\n' >> "$f"
                    ;;
                zsh)
                    f="$HOME/.zshrc"
                    grep -q '\.local/bin' "$f" 2>/dev/null || \
                        printf 'export PATH="$HOME/.local/bin:$PATH"\n' >> "$f"
                    ;;
                *)
                    f="$HOME/.bashrc"
                    grep -q '\.local/bin' "$f" 2>/dev/null || \
                        printf 'export PATH="$HOME/.local/bin:$PATH"\n' >> "$f"
                    f="$HOME/.profile"
                    grep -q '\.local/bin' "$f" 2>/dev/null || \
                        printf 'export PATH="$HOME/.local/bin:$PATH"\n' >> "$f"
                    ;;
            esac
            warn "PATH update lands in new terminals (shell: $shell)"
            ;;
    esac
fi

# -------------------------------------------------------------- verification ---
log "verifying the daemon"
SOCK="${XDG_RUNTIME_DIR:-/tmp}/chrona.sock"
up=false
for _ in $(seq 1 15); do
    if dbus-send --session --print-reply=literal --dest=org.chrona.Watcher \
            /org/chrona/Watcher org.chrona.Watcher.Ping 2>/dev/null | grep -q chrona; then
        up=true; break
    fi
    [ -S "$SOCK" ] && up=true && break
    sleep 1
done

sock_status() {
    if command -v socat >/dev/null; then
        echo '{"id":1,"cmd":"status"}' | socat - "UNIX-CONNECT:$SOCK" 2>/dev/null || true
    elif command -v python3 >/dev/null; then
        python3 - "$SOCK" <<'PYEOF'
import socket, sys
try:
    s = socket.socket(socket.AF_UNIX); s.settimeout(3)
    s.connect(sys.argv[1]); s.sendall(b'{"id":1,"cmd":"status"}')
    print(s.makefile("r").readline().strip())
except Exception:
    pass
PYEOF
    fi
}

if $up; then
    ok "daemon is up (socket $SOCK)"
else
    warn "daemon did not come up yet — check: systemctl --user status chrona"
fi

# End-to-end test: push a window event exactly like the KWin script does and
# confirm it shows up in status.current_window.
if $IS_KDE; then
    log "end-to-end test (window event -> status)"
    if dbus-send --session --print-reply=literal --dest=org.chrona.Watcher \
            /org/chrona/Watcher org.chrona.Watcher.ActiveWindowChanged \
            string:"chrona" string:"chrona" string:"installer-check" >/dev/null 2>&1; then
        sleep 1
        if sock_status | grep -q 'installer-check'; then
            ok "window events flow into the daemon — recording works"
        else
            warn "the event was accepted but status did not show it — try: systemctl --user restart chrona"
        fi
    else
        warn "could not reach org.chrona.Watcher over D-Bus"
    fi
fi

echo
printf '%s%sChrona %s installed.%s\n' "$BOLD" "$GREEN" "$VER" "$RST"
printf '  binaries   %s/chrona  and  %s/chronad\n' "$BIN" "$BIN"
[ -z "$AUTOSTART" ] && printf '  service    systemctl --user status chrona\n' \
                    || printf '  service    ~/.config/autostart/chronad.desktop\n'
$KWIN_PKG_DONE && printf '  kwin       watcher script enabled\n'
printf '  launch     %schrona%s  (dashboard)\n' "$BOLD" "$RST"
[ -z "$AUTOSTART" ] || printf '\n  note: without systemd the daemon was started for this session only.\n'
