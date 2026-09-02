//! AFK / idle detection backends. Idle data is what turns raw window events
//! into honest "screen time": time away from the machine is subtracted.

use crate::state::Event;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Poll `org.freedesktop.ScreenSaver.GetActive` via a cached `zbus`
/// connection. KDE Plasma and GNOME both implement the interface, so this is
/// the fallback for any session with a D-Bus bus. Resolution: one poll
/// interval. (v0.2 spawned a `dbus-send` subprocess on every poll; the
/// connection is now created once per thread and re-created only if the
/// bus goes away.)
#[cfg(feature = "dbus")]
pub fn spawn_dbus_screensaver(tx: Sender<Event>, poll_secs: u64) {
    std::thread::Builder::new()
        .name("chrona-idle-dbus".into())
        .spawn(move || screensaver_loop(&tx, poll_secs, None))
        .ok();
}

/// `dbus-send` fallback used when built without the `dbus` feature.
#[cfg(not(feature = "dbus"))]
pub fn spawn_dbus_screensaver(tx: Sender<Event>, poll_secs: u64) {
    std::thread::Builder::new()
        .name("chrona-idle-dbus".into())
        .spawn(move || {
            let mut active: Option<bool> = None;
            loop {
                if let Some(state) = screensaver_active_subprocess() {
                    if active != Some(state) {
                        let _ = tx.send(if state {
                            Event::IdleStart
                        } else {
                            Event::IdleEnd
                        });
                        active = Some(state);
                    }
                }
                std::thread::sleep(Duration::from_secs(poll_secs));
            }
        })
        .ok();
}

#[cfg(feature = "dbus")]
fn screensaver_active(conn: &mut Option<zbus::blocking::Connection>) -> Option<bool> {
    let call = |c: &zbus::blocking::Connection| -> zbus::Result<bool> {
        let proxy = zbus::blocking::Proxy::new(
            c,
            "org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver",
        )?;
        let reply = proxy.call_method("GetActive", &())?;
        let active: bool = reply.body().deserialize()?;
        Ok(active)
    };
    if conn.is_none() {
        *conn = zbus::blocking::Connection::session().ok();
    }
    match conn.as_ref().and_then(|c| call(c).ok()) {
        Some(v) => Some(v),
        // Bus restarted or daemon died: drop the connection and retry once
        // on the next poll with a fresh one.
        None => {
            *conn = None;
            None
        }
    }
}

#[cfg(not(feature = "dbus"))]
fn screensaver_active_subprocess() -> Option<bool> {
    let out = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.ScreenSaver",
            "/org/freedesktop/ScreenSaver",
            "org.freedesktop.ScreenSaver.GetActive",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout);
    Some(s.contains("boolean true"))
}

/// GNOME idle detection via Mutter's IdleMonitor: reports milliseconds
/// since the last user input (same semantics as the X11 MIT-SCREEN-SAVER
/// poll), so AFK starts at `threshold_ms` of inactivity rather than only
/// when the screen locks. Falls back to the screensaver poll if Mutter
/// does not answer (older GNOME, or a non-GNOME session).
#[cfg(feature = "dbus")]
pub fn spawn_mutter_idle(tx: Sender<Event>, threshold_ms: u32, poll_secs: u64) {
    std::thread::Builder::new()
        .name("chrona-idle-mutter".into())
        .spawn(move || {
            let mut conn: Option<zbus::blocking::Connection> = None;
            let mut idle = false;
            let mut mutter_failures = 0u32;
            loop {
                let ms = mutter_idletime_ms(&mut conn);
                match ms {
                    Some(ms) => {
                        mutter_failures = 0;
                        let now_idle = ms >= threshold_ms;
                        if now_idle != idle {
                            idle = now_idle;
                            let _ = tx.send(if now_idle {
                                Event::IdleStart
                            } else {
                                Event::IdleEnd
                            });
                        }
                    }
                    None => {
                        // Mutter is not answering (older GNOME, or the
                        // session is not GNOME after all): after a few
                        // failures degrade permanently to the generic
                        // screensaver poll on this same thread.
                        mutter_failures += 1;
                        if mutter_failures == 3 {
                            eprintln!(
                                "[chronad] org.gnome.Mutter.IdleMonitor unresponsive — falling back to ScreenSaver polling"
                            );
                            return screensaver_loop(&tx, poll_secs, conn);
                        }
                    }
                }
                std::thread::sleep(Duration::from_secs(poll_secs));
            }
        })
        .ok();
}

/// Shared poll loop for the screensaver fallback.
#[cfg(feature = "dbus")]
fn screensaver_loop(
    tx: &Sender<Event>,
    poll_secs: u64,
    mut conn: Option<zbus::blocking::Connection>,
) {
    let mut active: Option<bool> = None;
    loop {
        if let Some(state) = screensaver_active(&mut conn) {
            if active != Some(state) {
                let _ = tx.send(if state {
                    Event::IdleStart
                } else {
                    Event::IdleEnd
                });
                active = Some(state);
            }
        }
        std::thread::sleep(Duration::from_secs(poll_secs));
    }
}

#[cfg(feature = "dbus")]
fn mutter_idletime_ms(conn: &mut Option<zbus::blocking::Connection>) -> Option<u32> {
    let call = |c: &zbus::blocking::Connection| -> zbus::Result<u32> {
        let proxy = zbus::blocking::Proxy::new(
            c,
            "org.gnome.Mutter.IdleMonitor",
            "/org/gnome/Mutter/IdleMonitor/Core",
            "org.gnome.Mutter.IdleMonitor",
        )?;
        let reply = proxy.call_method("GetIdletime", &())?;
        let ms: u32 = reply.body().deserialize()?;
        Ok(ms)
    };
    if conn.is_none() {
        *conn = zbus::blocking::Connection::session().ok();
    }
    match conn.as_ref().and_then(|c| call(c).ok()) {
        Some(v) => Some(v),
        None => {
            *conn = None;
            None
        }
    }
}

/// X11 idle detection via the MIT-SCREEN-SAVER extension: reports
/// milliseconds since the last user input. One connection per poll keeps the
/// code simple and survives X server restarts.
#[cfg(feature = "x11")]
pub fn spawn_x11(tx: Sender<Event>, threshold_ms: u32, poll_secs: u64) {
    use x11rb::connection::Connection;
    use x11rb::protocol::screensaver::ConnectionExt as _;
    use x11rb::rust_connection::RustConnection;

    std::thread::Builder::new()
        .name("chrona-idle-x11".into())
        .spawn(move || {
            let mut idle = false;
            loop {
                let ms_since_input = (|| -> Option<u32> {
                    let (conn, screen) = RustConnection::connect(None).ok()?;
                    let root = conn.setup().roots[screen].root;
                    let info = conn.screensaver_query_info(root).ok()?.reply().ok()?;
                    Some(info.ms_since_user_input)
                })();
                if let Some(ms) = ms_since_input {
                    let now_idle = ms >= threshold_ms;
                    if now_idle != idle {
                        idle = now_idle;
                        let _ = tx.send(if now_idle {
                            Event::IdleStart
                        } else {
                            Event::IdleEnd
                        });
                    }
                }
                std::thread::sleep(Duration::from_secs(poll_secs));
            }
        })
        .ok();
}
