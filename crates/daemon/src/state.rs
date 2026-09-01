//! The recording state machine: turns a stream of window/idle events into
//! `WindowEvent` and `AfkSession` rows.

use chrona_core::model::{AfkSession, WindowEvent};
use chrona_store::Store;
use std::sync::mpsc::Receiver;
use std::sync::Mutex;
use std::time::Duration;

pub enum Event {
    /// The active (focused) window changed.
    Window { app: String, title: String },
    /// The user went idle / the session got locked.
    IdleStart,
    /// The user came back.
    IdleEnd,
    /// Periodic persistence tick — extends the currently open event.
    Flush,
    /// Graceful shutdown.
    Shutdown,
    /// Recording was paused / resumed by the user (privacy switch).
    SetPaused(bool),
}

pub struct Tracker {
    cur: Option<(i64, WindowEvent)>,
    last_window: Option<(String, String)>,
    afk: Option<(i64, AfkSession)>,
    paused: bool,
}

impl Tracker {
    pub fn new() -> Self {
        Self {
            cur: None,
            last_window: None,
            afk: None,
            paused: false,
        }
    }

    pub fn is_afk(&self) -> bool {
        self.afk.is_some()
    }

    pub fn current_window(&self) -> Option<(String, String)> {
        self.cur
            .as_ref()
            .map(|(_, e)| (e.app_id.clone(), e.title.clone()))
    }

    pub fn handle(&mut self, ev: Event, store: &Store, now: i64) {
        match ev {
            Event::Window { app, title } => {
                if self.paused {
                    // Recording is paused: ignore the desktop entirely.
                    return;
                }
                // A window event implies user input: if we were AFK, end it.
                if let Some((id, _)) = self.afk.take() {
                    let _ = store.update_afk_end(id, now);
                }
                let same = self
                    .cur
                    .as_ref()
                    .map(|(_, e)| e.same_window(&app, &title))
                    .unwrap_or(false);
                if !same {
                    self.close_current(store, now);
                    let e = WindowEvent::new(now, now, app.clone(), title.clone());
                    let id = store.insert_event(&e).unwrap_or(-1);
                    self.cur = Some((id, e));
                    self.last_window = Some((app, title));
                }
            }
            Event::IdleStart => {
                if self.paused {
                    return; // already recording nothing
                }
                if self.afk.is_none() {
                    self.close_current(store, now);
                    let a = AfkSession {
                        start: now,
                        end: now,
                    };
                    let id = store.insert_afk(&a).unwrap_or(-1);
                    self.afk = Some((id, a));
                }
            }
            Event::IdleEnd => {
                if self.paused {
                    return; // stay paused, AFK keeps accruing
                }
                if let Some((id, _)) = self.afk.take() {
                    let _ = store.update_afk_end(id, now);
                    // Re-open the last known window so time keeps flowing
                    // after an idle period that no watcher reported a focus
                    // change for (common on KDE after unlock).
                    if let Some((app, title)) = self.last_window.clone() {
                        let e = WindowEvent::new(now, now, app, title);
                        let id = store.insert_event(&e).unwrap_or(-1);
                        self.cur = Some((id, e));
                    }
                }
            }
            Event::Flush => {
                if self.afk.is_none() {
                    if let Some((id, e)) = &mut self.cur {
                        e.end = now;
                        let _ = store.update_event_end(*id, now);
                    }
                } else if let Some((id, _)) = &self.afk {
                    let _ = store.update_afk_end(*id, now);
                }
            }
            Event::SetPaused(p) => {
                if p == self.paused {
                    return;
                }
                self.paused = p;
                if p {
                    // Stop recording: close the open window and count the
                    // paused span as AFK so it never lands in the stats.
                    self.close_current(store, now);
                    if self.afk.is_none() {
                        let a = AfkSession {
                            start: now,
                            end: now,
                        };
                        let id = store.insert_afk(&a).unwrap_or(-1);
                        self.afk = Some((id, a));
                    }
                } else if let Some((id, _)) = self.afk.take() {
                    let _ = store.update_afk_end(id, now);
                    // Resume with the last known window, mirroring IdleEnd.
                    if let Some((app, title)) = self.last_window.clone() {
                        let e = WindowEvent::new(now, now, app, title);
                        let id = store.insert_event(&e).unwrap_or(-1);
                        self.cur = Some((id, e));
                    }
                }
            }
            Event::Shutdown => {
                self.close_current(store, now);
                if let Some((id, _)) = self.afk.take() {
                    let _ = store.update_afk_end(id, now);
                }
            }
        }
    }

    fn close_current(&mut self, store: &Store, now: i64) {
        if let Some((id, _)) = self.cur.take() {
            let _ = store.update_event_end(id, now);
        }
    }
}

/// Main state loop. Checks the shutdown flag and the pause switch at least
/// once per second.
///
/// The tracker is the *shared* one (behind `api::Shared`'s mutex), so `status`
/// and the live UI always see the real current window instead of a stale
/// copy. Events arrive at human speed and `handle` writes at most one small
/// SQLite row, so holding the lock per event is cheap.
pub fn run(
    tracker: &Mutex<Tracker>,
    store: Store,
    rx: Receiver<Event>,
    shutdown: &'static std::sync::atomic::AtomicBool,
    paused: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;
    // Apply a persisted pause from a previous run on startup.
    let mut last_paused = paused.load(Ordering::Relaxed);
    if last_paused {
        tracker
            .lock()
            .unwrap()
            .handle(Event::SetPaused(true), &store, now());
    }
    loop {
        let p = paused.load(Ordering::Relaxed);
        if p != last_paused {
            tracker
                .lock()
                .unwrap()
                .handle(Event::SetPaused(p), &store, now());
            last_paused = p;
        }
        let ev = match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(ev) => ev,
            Err(_) => {
                if shutdown.load(Ordering::SeqCst) {
                    tracker
                        .lock()
                        .unwrap()
                        .handle(Event::Shutdown, &store, now());
                    return;
                }
                continue;
            }
        };
        let stop = matches!(ev, Event::Shutdown);
        tracker.lock().unwrap().handle(ev, &store, now());
        if stop {
            return;
        }
    }
}

pub fn now() -> i64 {
    chrono::Local::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn tmp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("chronad-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(&dir.join("t.db")).unwrap()
    }

    fn drive(tag: &str, events: Vec<Event>) -> (Store, Tracker) {
        let store = tmp_store(tag);
        let mut t = Tracker::new();
        for (i, ev) in events.into_iter().enumerate() {
            t.handle(ev, &store, 1000 + i as i64);
        }
        t.handle(Event::Shutdown, &store, 9999);
        (store, t)
    }

    #[test]
    fn window_switch_closes_previous_event() {
        let (store, _) = drive(
            "switch",
            vec![
                Event::Window {
                    app: "firefox".into(),
                    title: "a".into(),
                },
                Event::Window {
                    app: "code".into(),
                    title: "b".into(),
                },
            ],
        );
        let ev = store.events_range(0, i64::MAX).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].app_id, "firefox");
        assert_eq!(ev[0].end, 1001); // closed when the next window appeared
        assert_eq!(ev[1].app_id, "code");
        assert_eq!(ev[1].end, 9999); // closed by Shutdown
    }

    #[test]
    fn same_window_repeated_is_one_event() {
        let (store, _) = drive(
            "same",
            vec![
                Event::Window {
                    app: "firefox".into(),
                    title: "a".into(),
                },
                Event::Window {
                    app: "firefox".into(),
                    title: "a".into(),
                },
                Event::Flush,
            ],
        );
        let ev = store.events_range(0, i64::MAX).unwrap();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].end, 9999); // still open until Shutdown closes it
    }

    #[test]
    fn idle_closes_event_and_resume_reopens_last_window() {
        let (store, _) = drive(
            "idle",
            vec![
                Event::Window {
                    app: "firefox".into(),
                    title: "a".into(),
                },
                Event::IdleStart,
                Event::IdleEnd,
            ],
        );
        let ev = store.events_range(0, i64::MAX).unwrap();
        let afk = store.afk_range(0, i64::MAX).unwrap();
        assert_eq!(ev.len(), 2); // closed at idle, reopened at resume
        assert_eq!(ev[0].end, 1001);
        assert_eq!(ev[1].app_id, "firefox");
        assert_eq!(ev[1].start, 1002);
        assert_eq!(afk.len(), 1);
        assert_eq!(afk[0].start, 1001);
        assert_eq!(afk[0].end, 1002);
    }

    #[test]
    fn pause_stops_recording_and_counts_away() {
        let (store, _) = drive(
            "pause",
            vec![
                Event::Window {
                    app: "firefox".into(),
                    title: "a".into(),
                },
                Event::SetPaused(true),
                Event::Window {
                    app: "steam".into(),
                    title: "ignored".into(),
                },
                Event::Flush,
                Event::SetPaused(false),
            ],
        );
        let ev = store.events_range(0, i64::MAX).unwrap();
        let afk = store.afk_range(0, i64::MAX).unwrap();
        // The steam window while paused must never be recorded.
        assert!(ev.iter().all(|e| e.app_id == "firefox"));
        // Pause span is stored as AFK (away), not usage.
        assert_eq!(afk.len(), 1);
        assert_eq!(afk[0].start, 1001); // began when pause started
        assert_eq!(afk[0].end, 1004); // ended when recording resumed
    }

    #[test]
    fn channel_end_to_end() {
        static SHUTDOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();
        let store = tmp_store("e2e");
        let s2 = Store::open(store.path()).unwrap();
        let paused: std::sync::Arc<std::sync::atomic::AtomicBool> =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let tracker = std::sync::Mutex::new(Tracker::new());
        let handle = std::thread::spawn(move || {
            run(&tracker, s2, rx, &SHUTDOWN, paused);
        });
        tx.send(Event::Window {
            app: "x".into(),
            title: "y".into(),
        })
        .unwrap();
        tx.send(Event::Shutdown).unwrap();
        handle.join().unwrap();
        assert_eq!(store.events_range(0, i64::MAX).unwrap().len(), 1);
    }
}
