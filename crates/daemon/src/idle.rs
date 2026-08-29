//! AFK / idle detection backends. Idle data is what turns raw window events
//! into honest "screen time": time away from the machine is subtracted.

use crate::state::Event;
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Poll `org.freedesktop.ScreenSaver.GetActive` via `dbus-send`. KDE Plasma
/// and GNOME both implement the interface, so this is the fallback for any
/// session with a D-Bus bus. Resolution: one poll interval.
pub fn spawn_dbus_screensaver(tx: Sender<Event>, poll_secs: u64) {
    std::thread::Builder::new()
        .name("chrona-idle-dbus".into())
        .spawn(move || {
            let mut active: Option<bool> = None;
            loop {
                if let Some(state) = screensaver_active() {
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

fn screensaver_active() -> Option<bool> {
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
