// Chrona GNOME Shell watcher — reports the focused window to chronad over D-Bus.
//
// GNOME's Mutter (Wayland) does not implement the wlr-foreign-toplevel
// protocol, and unlike KWin there is no supported scripting hook — the only
// sanctioned window-event source inside GNOME Shell is an extension. This
// extension is that source: it fires on every focus change (and on title
// changes of the focused window, e.g. switching browser tabs) and calls
// org.chrona.Watcher.ActiveWindowChanged with (wm_class_instance,
// wm_class, title). Everything stays on the session bus — nothing leaves
// the machine.
//
// Mirrors kwin/chrona-watcher/contents/code/main.js.
//
// SPDX-License-Identifier: MIT

import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

const BUS = 'org.chrona.Watcher';
const PATH = '/org/chrona/Watcher';
const IFACE = 'org.chrona.Watcher';
const METHOD = 'ActiveWindowChanged';

export default class ChronaWatcherExtension {
    enable() {
        this._titleId = 0;
        this._titleWin = null;
        this._focusId = global.display.connect('notify::focus-window', () => this._onFocus());
        this._onFocus();
    }

    disable() {
        if (this._focusId) {
            global.display.disconnect(this._focusId);
            this._focusId = 0;
        }
        this._detachTitle();
    }

    _onFocus() {
        // Follow the focused window: title changes on it (browser tab
        // switches, terminal renames) are separate events for Chrona's
        // per-title tracking.
        this._detachTitle();
        const win = global.display.get_focus_window();
        if (win) {
            try {
                this._titleId = win.connect('notify::title', () => this._report());
                this._titleWin = win;
            } catch (e) {
                // Property notify not available? focus events still work.
            }
        }
        this._report();
    }

    _detachTitle() {
        if (this._titleId && this._titleWin) {
            this._titleWin.disconnect(this._titleId);
        }
        this._titleId = 0;
        this._titleWin = null;
    }

    _report() {
        const win = global.display.get_focus_window();
        if (!win) {
            return;
        }
        // resource_name ≈ wm-class instance, resource_class ≈ wm class
        // (the app id chrona matches rules against).
        let cls = '';
        let inst = '';
        try {
            cls = win.get_wm_class() || '';
            inst = win.get_wm_class_instance() || '';
        } catch (e) {
            // MetaWindow API changed? fall through with empty strings.
        }
        if (!cls) {
            // GTK4/Adwaita apps set the application id instead of a wm
            // class — use it so "org.gnome.Nautilus" still categorises.
            try {
                cls = win.get_gtk_application_id() || '';
            } catch (e) {
            }
        }
        let title = '';
        try {
            title = win.get_title() || '';
        } catch (e) {
        }
        if (!cls && !inst) {
            return; // nothing to identify the window by
        }
        try {
            Gio.DBus.session.call(
                BUS,
                PATH,
                IFACE,
                METHOD,
                new GLib.Variant('(sss)', [
                    String(inst || '').toLowerCase(),
                    String(cls || '').toLowerCase(),
                    String(title),
                ]),
                null,
                Gio.DBusCallFlags.NONE,
                -1,
                null,
                null
            );
        } catch (e) {
            // Daemon not up (yet) — extensions must never throw uncaught.
            log(`[chrona-watcher] could not reach chronad: ${e.message}`);
        }
    }
}
