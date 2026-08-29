//! Compositor detection: which watcher backend should record window events?

#[cfg(feature = "wayland")]
pub mod wayland;
#[cfg(feature = "x11")]
pub mod x11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// KDE Plasma on Wayland — KWin script pushes events over D-Bus.
    KdeWayland,
    /// wlroots-based compositors (Sway, Hyprland, river, niri...) via
    /// the wlr-foreign-toplevel protocol.
    WlrootsWayland,
    /// Classic X11 — EWMH polling.
    X11,
    /// Anything else (e.g. GNOME Wayland) — window events unavailable in
    /// v0.2, see docs/WATCHERS.md.
    Unsupported,
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
    SessionKind::Unsupported
}
