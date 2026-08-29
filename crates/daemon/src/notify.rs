//! Daily-limit notifications — the feature RescueTime and Google Digital
//! Wellbeing both ship, but with a sequence instead of a single ping:
//!
//! 1. **Heads-up at 90 %** ("12m left of 1h") — a chance to wind down before
//!    the wall, the moment Android's "5 minutes left" toast fires.
//! 2. **Limit reached** (critical urgency) — once per goal per day.
//! 3. **Nag while over** — every `nag_minutes` (default 15, `0` disables)
//!    a low-key reminder of how far past the limit you are.
//!
//! All checks run against today's AFK-subtracted usage — exactly the numbers
//! the dashboard shows — every 30 s via `org.freedesktop.Notifications` on
//! the session bus. Opt out with the `notify` setting
//! (`settings.set {"key":"notify","value":"0"}`). Headless systems without
//! a session bus are fine — sends are best-effort and failures are ignored.
//! A daemon restart never spams old news: the first pass only arms state.

use crate::api::{self, Shared};
use chrono::Local;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

/// How often limits are checked (keep well above the 10s flush interval so
/// usage numbers are settled before a notification fires).
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Fire the heads-up when this fraction of the limit is used.
const HEADS_UP_FRACTION: f64 = 0.9;

/// Don't send heads-ups for tiny limits where "90 %" is noise.
const HEADS_UP_MIN_LIMIT: i64 = 20 * 60;

#[derive(Default)]
struct GoalState {
    /// Heads-up (90 %) sent for today.
    warned: bool,
    /// Limit-reached notification sent for today.
    over: bool,
    /// Last nag timestamp; nags repeat every `nag_minutes`.
    last_nag: Option<Instant>,
}

pub fn spawn(shared: Arc<Shared>) {
    std::thread::Builder::new()
        .name("chrona-notify".into())
        .spawn(move || {
            // (goal id, date) → state. Re-armed if usage drops back under
            // the limit (e.g. after a purge) so the sequence can replay.
            let mut state: HashMap<String, GoalState> = HashMap::new();
            // First pass only arms already-exceeded goals so a daemon
            // restart never spams notifications for old news.
            let mut first_pass = true;
            loop {
                std::thread::sleep(CHECK_INTERVAL);
                if shared.store.setting("notify").ok().flatten().as_deref() == Some("0") {
                    continue;
                }
                let Ok(goals) = shared.store.goals() else {
                    continue;
                };
                let nag_minutes: i64 = shared
                    .store
                    .setting("nag_minutes")
                    .ok()
                    .flatten()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(15);
                let usage = api::today_usage_map(&shared);
                let date = Local::now().format("%Y-%m-%d").to_string();
                for g in goals.iter().filter(|g| g.enabled && g.limit_seconds > 0) {
                    let Some(used) = usage.get(&format!("{}:{}", g.kind, g.key)) else {
                        continue;
                    };
                    let over = *used > g.limit_seconds;
                    let near = *used as f64 >= g.limit_seconds as f64 * HEADS_UP_FRACTION;
                    let key = format!("{}:{date}", g.id);
                    let st = state.entry(key.clone()).or_default();

                    // ---- 2. limit reached (critical) ----
                    if over && !st.over {
                        st.over = true;
                        if !first_pass {
                            send_limit(&shared, g, *used);
                            st.last_nag = Some(Instant::now());
                        }
                    } else if !over {
                        // Usage dropped below (purge / clock rollover): re-arm.
                        st.over = false;
                        st.last_nag = None;
                        if !near {
                            st.warned = false;
                        }
                    }

                    // ---- 1. heads-up at 90 % ----
                    if near && !over && !st.warned && g.limit_seconds >= HEADS_UP_MIN_LIMIT {
                        st.warned = true;
                        if !first_pass {
                            send_heads_up(&shared, g, *used);
                        }
                    }

                    // ---- 3. nag while over ----
                    if over && nag_minutes > 0 {
                        let due = st
                            .last_nag
                            .map(|t| t.elapsed() >= Duration::from_secs(nag_minutes as u64 * 60))
                            .unwrap_or(true);
                        if due && !first_pass {
                            st.last_nag = Some(Instant::now());
                            send_nag(&shared, g, *used);
                        } else if due {
                            st.last_nag = Some(Instant::now());
                        }
                    }
                }
                first_pass = false;
            }
        })
        .ok();
}

fn fmt_hm(s: i64) -> String {
    if s >= 3600 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else {
        format!("{}m", s / 60)
    }
}

fn label_of(g: &chrona_store::Goal) -> String {
    match g.kind.as_str() {
        "total" => "Screen time".to_string(),
        "category" => chrona_core::model::Category::from_key(&g.key)
            .map(|c| c.display().to_string())
            .unwrap_or_else(|| g.key.clone()),
        _ => g.key.clone(),
    }
}

fn send_limit(_shared: &Shared, g: &chrona_store::Goal, used: i64) {
    let label = label_of(g);
    let title = if g.kind == "total" {
        "Screen time goal reached".to_string()
    } else {
        "Time limit reached".to_string()
    };
    let body = format!(
        "{} — {} of {} today. Time to wrap up.",
        label,
        fmt_hm(used),
        fmt_hm(g.limit_seconds)
    );
    notify(&title, &body, Urgency::Critical, 8000);
}

fn send_heads_up(_shared: &Shared, g: &chrona_store::Goal, used: i64) {
    let label = label_of(g);
    let left = (g.limit_seconds - used).max(0);
    let title = format!("{label}: almost there");
    let body = format!(
        "{} of {} used — about {} left today.",
        fmt_hm(used),
        fmt_hm(g.limit_seconds),
        fmt_hm(left)
    );
    notify(&title, &body, Urgency::Normal, 5000);
}

fn send_nag(_shared: &Shared, g: &chrona_store::Goal, used: i64) {
    let label = label_of(g);
    let over_by = used - g.limit_seconds;
    let title = format!("{label}: still over");
    let body = format!(
        "{} past the limit ({} of {}).",
        fmt_hm(over_by),
        fmt_hm(used),
        fmt_hm(g.limit_seconds)
    );
    notify(&title, &body, Urgency::Normal, 5000);
}

enum Urgency {
    Normal,
    Critical,
}

fn notify(title: &str, body: &str, urgency: Urgency, timeout_ms: i32) {
    #[cfg(feature = "dbus")]
    {
        use std::collections::HashMap;
        let Ok(conn) = zbus::blocking::Connection::session() else {
            return; // headless box, nothing to notify
        };
        let proxy = match zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[chronad] notification proxy failed: {e}");
                return;
            }
        };
        let mut hints = HashMap::<String, zbus::zvariant::Value>::new();
        hints.insert(
            "urgency".into(),
            zbus::zvariant::Value::U8(match urgency {
                Urgency::Normal => 1,
                Urgency::Critical => 2,
            }),
        );
        if let Err(e) = proxy.call_method(
            "Notify",
            &(
                "chronad",
                0u32,
                "chrona",
                title,
                body,
                Vec::<String>::new(),
                hints,
                timeout_ms,
            ),
        ) {
            eprintln!("[chronad] notification failed: {e}");
        }
    }
    #[cfg(not(feature = "dbus"))]
    {
        // Built without D-Bus support: limits still work, notifications
        // degrade to the dashboard banner.
        let _ = (title, body, urgency, timeout_ms);
    }
}
