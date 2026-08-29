//! Desktop notifications when a daily limit is reached — the feature
//! RescueTime and Google Digital Wellbeing both ship. Chrona checks every
//! goal against today's usage (AFK-subtracted, exactly what the dashboard
//! shows) and notifies once per goal per day via
//! `org.freedesktop.Notifications` on the session bus.
//!
//! Opt out with the `notify` setting (`settings.set {"key":"notify",
//! "value":"0"}`). Headless systems without a session bus are fine — the
//! send is best-effort and failures are ignored.

use crate::api::{self, Shared};
use chrono::Local;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

/// How often limits are checked (keep well above the 10s flush interval so
/// usage numbers are settled before a notification fires).
const CHECK_INTERVAL: Duration = Duration::from_secs(30);

pub fn spawn(shared: Arc<Shared>) {
    std::thread::Builder::new()
        .name("chrona-notify".into())
        .spawn(move || {
            // (goal id, date) keys already alerted — one notification per
            // goal per day, re-armed if usage drops back under the limit
            // (e.g. after a purge).
            let mut alerted: HashSet<String> = HashSet::new();
            // First pass only marks already-exceeded goals so a daemon
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
                let usage = api::today_usage_map(&shared);
                let date = Local::now().format("%Y-%m-%d").to_string();
                for g in goals.iter().filter(|g| g.enabled && g.limit_seconds > 0) {
                    let Some(used) = usage.get(&format!("{}:{}", g.kind, g.key)) else {
                        continue;
                    };
                    let over = *used > g.limit_seconds;
                    let key = format!("{}:{date}", g.id);
                    if over && !alerted.contains(&key) {
                        alerted.insert(key);
                        if !first_pass {
                            send(&shared, g, *used);
                        }
                    } else if !over {
                        alerted.remove(&key);
                    }
                }
                first_pass = false;
            }
        })
        .ok();
}

fn fmt_hm(s: i64) -> String {
    format!("{}h {}m", s / 3600, (s % 3600) / 60)
}

fn send(_shared: &Shared, g: &chrona_store::Goal, used: i64) {
    let label = match g.kind.as_str() {
        "total" => "Screen time".to_string(),
        "category" => chrona_core::model::Category::from_key(&g.key)
            .map(|c| c.display().to_string())
            .unwrap_or_else(|| g.key.clone()),
        _ => g.key.clone(),
    };
    let body = format!(
        "{} — {} of {} today.",
        label,
        fmt_hm(used),
        fmt_hm(g.limit_seconds)
    );
    let title = if g.kind == "total" {
        "Screen time goal reached".to_string()
    } else {
        "Time limit reached".to_string()
    };
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
        if let Err(e) = proxy.call_method(
            "Notify",
            &(
                "chronad",
                0u32,
                "chrona",
                title.as_str(),
                body.as_str(),
                Vec::<String>::new(),
                HashMap::<String, zbus::zvariant::Value>::new(),
                6000i32,
            ),
        ) {
            eprintln!("[chronad] notification failed: {e}");
        }
    }
    #[cfg(not(feature = "dbus"))]
    {
        // Built without D-Bus support: limits still work, notifications
        // degrade to the dashboard badge.
        let _ = (_shared, g, used);
    }
}
