//! Compositor detection: which watcher backend should record window events?

#[cfg(feature = "wayland")]
pub mod wayland;
#[cfg(feature = "x11")]
pub mod x11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// KDE Plasma on Wayland — KWin script pushes events over D-Bus.
    KdeWayland,
    /// GNOME Shell on Wayland — the Chrona extension pushes events over
    /// D-Bus (same intake as the KWin script).
    GnomeWayland,
    /// wlroots-based compositors (Sway, Hyprland, river, niri...) via
    /// the wlr-foreign-toplevel protocol.
    WlrootsWayland,
    /// Classic X11 — EWMH polling.
    X11,
    /// Anything else — window events unavailable, see docs/WATCHERS.md.
    Unsupported,
}

/// True when `XDG_CURRENT_DESKTOP` lists `GNOME` as one of its
/// colon-separated components (e.g. `ubuntu:GNOME`, plain `GNOME`).
/// Budgie also exports `Budgie:GNOME` but is NOT stock GNOME Shell —
/// exclude it explicitly so we do not promise a watcher it cannot load.
fn is_gnome_desktop() -> bool {
    desktop_has_gnome(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
}

/// Pure form of [`is_gnome_desktop`] — testable without env mutation.
fn desktop_has_gnome(desktop: &str) -> bool {
    // Single pass — two `.any()` calls would share one advancing iterator
    // and the second would only see components after the first match.
    let (mut gnome, mut budgie) = (false, false);
    for c in desktop.split(':').map(str::trim) {
        if c.eq_ignore_ascii_case("gnome") {
            gnome = true;
        }
        if c.eq_ignore_ascii_case("budgie") {
            budgie = true;
        }
    }
    gnome && !budgie
}

pub fn detect() -> SessionKind {
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if session_type == "x11" {
        return SessionKind::X11;
    }
    let wayland = session_type == "wayland" || std::env::var("WAYLAND_DISPLAY").is_ok();
    if !wayland {
        return SessionKind::Unsupported;
    }
    let kde = std::env::var("KDE_FULL_SESSION")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if kde {
        return SessionKind::KdeWayland;
    }
    let wlroots = [
        "SWAYSOCK",
        "HYPRLAND_INSTANCE_SIGNATURE",
        "NIRI_SOCKET",
        "RIVER_UNIX_SOCKET",
    ]
    .iter()
    .any(|v| std::env::var(v).map(|x| !x.is_empty()).unwrap_or(false));
    if wlroots {
        return SessionKind::WlrootsWayland;
    }
    if is_gnome_desktop() {
        return SessionKind::GnomeWayland;
    }
    SessionKind::Unsupported
}

#[cfg(test)]
mod tests {
    use super::desktop_has_gnome;

    #[test]
    fn gnome_desktop_detection() {
        assert!(desktop_has_gnome("GNOME"));
        assert!(desktop_has_gnome("ubuntu:GNOME"));
        assert!(desktop_has_gnome("GNOME-Flashback:GNOME"));
        // Substring matches must NOT count.
        assert!(!desktop_has_gnome("Budgie:GNOME")); // not stock GNOME Shell
        assert!(!desktop_has_gnome("X-Cinnamon"));
        assert!(!desktop_has_gnome("KDE"));
        assert!(!desktop_has_gnome(""));
        assert!(!desktop_has_gnome("unity:Unity7:ubuntu"));
    }
}
