//! X11 watcher: polls the EWMH `_NET_ACTIVE_WINDOW` property and reports
//! window class + title whenever the focused window changes.
#![cfg(feature = "x11")]

use crate::state::Event;
use std::sync::mpsc::Sender;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    pub Atoms:
    AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_NAME,
        WM_CLASS,
        WM_NAME,
        UTF8_STRING,
    }
}

pub struct X11Watcher {
    conn: RustConnection,
    screen_num: usize,
    atoms: Atoms,
    last: Option<(String, String)>,
}

/// How often `_NET_ACTIVE_WINDOW` is polled. This MUST stay in sync with
/// docs/WATCHERS.md ("EWMH polling (2 s resolution)") and the watcher label
/// in main.rs — all three say 2 s. Do not tune one without the others.
const POLL_INTERVAL: Duration = Duration::from_millis(2000);

impl X11Watcher {
    pub fn connect() -> anyhow::Result<Self> {
        let (conn, screen_num) = RustConnection::connect(None)?;
        let atoms = Atoms::new(&conn)?.reply()?;
        Ok(Self {
            conn,
            screen_num,
            atoms,
            last: None,
        })
    }

    fn root(&self) -> u32 {
        self.conn.setup().roots[self.screen_num].root
    }

    fn active_window(&self) -> Option<u32> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root(),
                self.atoms._NET_ACTIVE_WINDOW,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()?
            .reply()
            .ok()?;
        let win = reply.value32()?.next();
        win
    }

    fn string_prop(&self, window: u32, prop: u32, kind: u32) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, window, prop, kind, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        if reply.value.is_empty() {
            return None;
        }
        // WM_CLASS is "instance\0class\0" — we want the class (second part).
        let parts: Vec<&[u8]> = reply
            .value
            .split(|&b| b == 0)
            .filter(|p| !p.is_empty())
            .collect();
        let raw = parts.last()?;
        Some(String::from_utf8_lossy(raw).trim().to_string())
    }

    fn poll_once(&mut self) -> Option<(String, String)> {
        let win = self.active_window()?;
        if win == 0 {
            return None; // desktop / no window
        }
        let class = self
            .string_prop(win, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
            .map(|s| s.to_lowercase());
        let title = self
            .string_prop(win, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING)
            .or_else(|| self.string_prop(win, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()));
        Some((class?, title.unwrap_or_default()))
    }

    pub fn run(mut self, tx: Sender<Event>) {
        loop {
            match self.poll_once() {
                Some(cur) => {
                    if self.last.as_ref() != Some(&cur) {
                        self.last = Some(cur.clone());
                        let _ = tx.send(Event::Window {
                            app: cur.0,
                            title: cur.1,
                        });
                    }
                }
                None => {
                    // Connection may have died; try to reconnect.
                    match X11Watcher::connect() {
                        Ok(w) => {
                            let last = self.last.take();
                            self.conn = w.conn;
                            self.screen_num = w.screen_num;
                            self.atoms = w.atoms;
                            self.last = last;
                        }
                        Err(_) => std::thread::sleep(Duration::from_secs(5)),
                    }
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

pub fn spawn(tx: Sender<Event>) -> anyhow::Result<()> {
    let w = X11Watcher::connect()?;
    std::thread::Builder::new()
        .name("chrona-x11".into())
        .spawn(move || w.run(tx))?;
    Ok(())
}
