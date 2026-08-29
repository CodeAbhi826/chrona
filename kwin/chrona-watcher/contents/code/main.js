// Chrona KWin watcher — reports the active window to chronad over D-Bus.
//
// KDE's KWin does not implement the wlr-foreign-toplevel protocol, so on
// Plasma Wayland this script is the window event source. It fires on every
// window activation and calls org.chrona.Watcher.ActiveWindowChanged with
// (resource_name, resource_class, caption). Everything stays on the session
// bus — nothing leaves the machine.
//
// SPDX-License-Identifier: MIT

function report(client) {
    if (!client) {
        return;
    }
    try {
        callDBus(
            "org.chrona.Watcher",
            "/org/chrona/Watcher",
            "org.chrona.Watcher",
            "ActiveWindowChanged",
            String(client.resourceName ?? ""),
            String(client.resourceClass ?? ""),
            String(client.caption ?? "")
        );
    } catch (e) {
        // Daemon not up (yet) — KWin scripts must never throw uncaught.
        print("[chrona-watcher] could not reach chronad:", e);
    }
}

workspace.windowActivated.connect(report);

// Report the currently focused window at script load: covers KWin restarts
// and re-loads without waiting for the next focus change.
report(workspace.activeWindow);
