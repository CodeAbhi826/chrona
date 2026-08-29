//! chronad — the Chrona daemon.
//!
//! Layout: watchers push `Event`s into a channel, `state::run` folds them
//! into the SQLite store, and `api::serve` answers queries on a local Unix
//! socket. One process, a few MB of RAM, no network — ever.

mod api;
#[cfg(feature = "dbus")]
mod dbus;
mod idle;
mod notify;
mod state;
mod watchers;

use chrona_store::Store;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
}

fn default_data_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".local/share")
        });
    base.join("chrona")
}

fn print_help() {
    println!(
        "chronad {} — Chrona background recorder\n\n\
         USAGE:\n    chronad [OPTIONS]\n\n\
         OPTIONS:\n    --socket <PATH>   Unix socket path (default: $XDG_RUNTIME_DIR/chrona.sock)\n    --data-dir <DIR>  Data directory   (default: $XDG_DATA_HOME/chrona)\n    --version         Print version\n    --help            Print this help",
        env!("CARGO_PKG_VERSION")
    );
}

fn main() -> anyhow::Result<()> {
    let mut socket_override: Option<PathBuf> = None;
    let mut data_dir_override: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("chronad {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            "--socket" => socket_override = args.next().map(PathBuf::from),
            "--data-dir" => data_dir_override = args.next().map(PathBuf::from),
            other => anyhow::bail!("unknown argument: {other} (see --help)"),
        }
    }

    install_signal_handlers();
    let data_dir = data_dir_override.unwrap_or_else(default_data_dir);
    let store = Store::open(&data_dir.join("chrona.db"))?;
    let socket_path = socket_override.unwrap_or_else(api::default_socket_path);

    let (tx, rx) = mpsc::channel::<state::Event>();
    let shared = Arc::new(api::Shared::new(store)?);

    // D-Bus intake: receives events from the Chrona KWin script (and manual
    // dbus-send for power users). Harmless if no session bus exists.
    #[cfg(feature = "dbus")]
    dbus::spawn_intake(tx.clone());

    // Compositor-specific window watching.
    let session = watchers::detect();
    let (watcher_label, idle_label) = match session {
        watchers::SessionKind::KdeWayland => {
            idle::spawn_dbus_screensaver(tx.clone(), 15);
            (
                "KDE Plasma Wayland — KWin script via D-Bus (install: docs/WATCHERS.md)"
                    .to_string(),
                "org.freedesktop.ScreenSaver poll (15s)".to_string(),
            )
        }
        watchers::SessionKind::WlrootsWayland => {
            let label = if watchers::wayland::spawn(tx.clone()).is_ok() {
                "wlroots Wayland — wlr-foreign-toplevel + ext-idle-notify".to_string()
            } else {
                "Wayland, but no wlr-foreign-toplevel — window events unavailable".to_string()
            };
            (label, "ext-idle-notify-v1".to_string())
        }
        watchers::SessionKind::X11 => {
            watchers::x11::spawn(tx.clone())?;
            idle::spawn_x11(tx.clone(), 60_000, 15);
            (
                "X11 — EWMH _NET_ACTIVE_WINDOW polling (2s)".to_string(),
                "MIT-SCREEN-SAVER poll (15s)".to_string(),
            )
        }
        watchers::SessionKind::Unsupported => {
            idle::spawn_dbus_screensaver(tx.clone(), 15);
            (
                "unsupported compositor — window events unavailable in v0.2 (see docs)".to_string(),
                "org.freedesktop.ScreenSaver poll (15s)".to_string(),
            )
        }
    };
    shared.set_watcher_label(&watcher_label);
    shared.set_idle_label(&idle_label);
    eprintln!("[chronad] watcher: {watcher_label}");
    eprintln!("[chronad] idle:    {idle_label}");

    // Periodic flush so the open event's end time stays fresh (a crash loses
    // at most one flush interval).
    {
        let tx = tx.clone();
        std::thread::Builder::new()
            .name("chrona-flush".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(10));
                if tx.send(state::Event::Flush).is_err() {
                    break;
                }
            })?;
    }

    // Local API socket.
    {
        let shared = Arc::clone(&shared);
        let path = socket_path.clone();
        std::thread::Builder::new()
            .name("chrona-api".into())
            .spawn(move || {
                if let Err(e) = api::serve(path, shared) {
                    eprintln!("[chronad] api server error: {e}");
                }
            })?;
    }

    // Daily-limit notifications (best-effort; no session bus → no-op).
    notify::spawn(Arc::clone(&shared));

    eprintln!(
        "[chronad] v{} ready — socket at {}",
        env!("CARGO_PKG_VERSION"),
        socket_path.display()
    );

    // The state machine uses its own connection to the same SQLite file
    // (WAL handles concurrent readers/writers). The pause switch is shared
    // with the API so `pause.set` takes effect within a second.
    let tracker_store = Store::open(shared.store.path())?;
    let paused = Arc::clone(&shared.paused);
    state::run(state::Tracker::new(), tracker_store, rx, &SHUTDOWN, paused);

    let _ = std::fs::remove_file(&socket_path);
    eprintln!("[chronad] stopped cleanly");
    Ok(())
}
