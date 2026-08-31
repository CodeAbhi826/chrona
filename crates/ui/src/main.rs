//! Chrona — the UI. Talks to `chronad` over a local Unix socket; if the
//! daemon is asleep, the window says so and shows how to start it.

#![cfg(target_os = "linux")]

mod client;

use client::REFRESH;
use serde_json::json;
use slint::ComponentHandle;
use std::io::Write as _;
use std::sync::atomic::Ordering;

slint::include_modules!();

/// Bundled Inter (SIL OFL 1.1) — the metric-friendly stand-in for Google
/// Sans, which cannot be legally redistributed with the app.
const INTER_FONT: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");

fn google_sans_installed() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = vec![
        std::path::PathBuf::from("/usr/share/fonts"),
        std::path::PathBuf::from(format!("{home}/.local/share/fonts")),
        std::path::PathBuf::from(format!("{home}/.fonts")),
    ];
    if let Ok(extra) = std::env::var("XDG_DATA_DIRS") {
        dirs.extend(std::env::split_paths(&extra).map(|p| p.join("fonts")));
    }
    fn scan(dir: &std::path::Path) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if (name.contains("google") && name.contains("sans")) || name.contains("productsans") {
                return true;
            }
            if e.path().is_dir() && scan(&e.path()) {
                return true;
            }
        }
        false
    }
    dirs.iter().any(|d| scan(d))
}

fn setup_fonts() -> &'static str {
    if google_sans_installed() {
        return "Google Sans";
    }
    // Extract the bundled Inter (OFL) so Slint can load it as the primary
    // font; SLINT_DEFAULT_FONT must be set before the first window is created.
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(&home).join(".cache"));
    let dir = cache.join("chrona");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("Inter.ttf");
    if std::fs::write(&path, INTER_FONT).is_ok() {
        std::env::set_var("SLINT_DEFAULT_FONT", &path);
    }
    "Inter"
}

fn main() -> Result<(), slint::PlatformError> {
    let family = setup_fonts();
    let app = ChronaApp::new()?;
    app.set_font_family(family.into());

    // ---- daemon polling ------------------------------------------------------
    // v0.2.0 spawned a std::thread that called `weak.upgrade()` off-thread;
    // slint::Weak::upgrade() returns None from any other thread, so the UI
    // never saw the daemon and showed "not running" forever. Timers run their
    // callbacks on the event-loop thread, where the upgrade is valid.
    let slow_timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        slow_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_secs(5),
            move || {
                if let Some(app) = weak.upgrade() {
                    client::tick(&app);
                }
            },
        );
    }
    // Buttons (add/remove goal, theme…) set REFRESH to ask for an early tick;
    // this timer honours that within 250 ms instead of waiting for the 5 s one.
    let fast_timer = slint::Timer::default();
    {
        let weak = app.as_weak();
        fast_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(250),
            move || {
                if REFRESH.swap(false, Ordering::SeqCst) {
                    if let Some(app) = weak.upgrade() {
                        client::tick(&app);
                    }
                }
            },
        );
    }
    client::tick(&app); // first tick on the main thread, before the loop starts

    // ---- theme ----
    {
        let weak = app.as_weak();
        app.on_toggle_theme(move |dark| {
            if let Some(app) = weak.upgrade() {
                app.global::<crate::Theme>().set_dark(dark);
                let _ = client::request(
                    "settings.set",
                    json!({"key": "theme", "value": if dark { "dark" } else { "material" }}),
                );
            }
        });
    }

    // ---- pause tracking ----
    {
        let weak = app.as_weak();
        app.on_toggle_pause(move |p| {
            if let Some(app) = weak.upgrade() {
                app.set_tracking_paused(p); // optimistic; poll keeps it honest
                let _ = client::request("pause.set", json!({"paused": p}));
            }
        });
    }

    // ---- apps view ----
    {
        let weak = app.as_weak();
        app.on_search_apps(move |q| {
            if let Some(app) = weak.upgrade() {
                client::apply_search(&app, &q);
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_select_app(move |id| {
            if let Some(app) = weak.upgrade() {
                client::load_app_detail(&app, &id);
            }
        });
    }

    // ---- goals ----
    {
        let _weak = app.as_weak();
        app.on_add_goal(move |kind, key, minutes| {
            let limit = (minutes.max(1.0) * 60.0) as i64;
            let _ = client::request(
                "goal.set",
                json!({"kind": kind.to_string(), "key": key.to_string(), "limit_seconds": limit, "enabled": true}),
            );
            REFRESH.store(true, Ordering::SeqCst);
        });
    }
    {
        let weak = app.as_weak();
        app.on_goal_kind_changed(move |kind| {
            if let Some(app) = weak.upgrade() {
                client::apply_goal_suggestions(&app, &kind);
            }
        });
    }
    {
        let _weak = app.as_weak();
        app.on_remove_goal(move |id| {
            let _ = client::request("goal.del", json!({"id": id}));
            REFRESH.store(true, Ordering::SeqCst);
        });
    }
    {
        let _weak = app.as_weak();
        app.on_toggle_goal(move |id, on| {
            // Re-set with new enabled flag: the daemon upserts by (kind,key),
            // so we first read the current goal, then write it back.
            if let Some(goals) = client::request("goals", json!({})) {
                if let Some(g) = goals.as_array().and_then(|a| {
                    a.iter().find(|g| {
                        g.get("id").and_then(serde_json::Value::as_i64) == Some(id as i64)
                    })
                }) {
                    let _ = client::request(
                        "goal.set",
                        json!({
                            "kind": g.get("kind").cloned().unwrap_or(json!("app")),
                            "key": g.get("key").cloned().unwrap_or(json!("")),
                            "limit_seconds": g.get("limit_seconds").cloned().unwrap_or(json!(3600)),
                            "enabled": on,
                        }),
                    );
                }
            }
            REFRESH.store(true, Ordering::SeqCst);
        });
    }

    // ---- settings actions ----
    {
        let weak = app.as_weak();
        app.on_install_kwin(move || {
            let status = install_kwin_script();
            if let Some(app) = weak.upgrade() {
                app.set_kwin_status(status.into());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_do_export(move || {
            let path = export_json();
            if let Some(app) = weak.upgrade() {
                app.set_export_path(path.into());
            }
        });
    }
    {
        let weak = app.as_weak();
        app.on_refresh(move || {
            let _ = weak;
            REFRESH.store(true, Ordering::SeqCst);
        });
    }

    app.run()
}

/// Copy the KWin watcher script into ~/.local/share/kwin/scripts and enable
/// it. Tries, in order: packaged script at /usr/share/chrona, the repo
/// checkout (cargo run), then gives up with a helpful message.
fn install_kwin_script() -> String {
    let candidates = [
        std::path::PathBuf::from("/usr/share/chrona/kwin/chrona-watcher"),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../kwin/chrona-watcher")
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent")),
    ];
    let Some(src) = candidates.iter().find(|p| p.exists()) else {
        return "KWin script not found — install the chrona package or run from the repo".into();
    };

    let home = std::env::var("HOME").unwrap_or_default();
    let dst = format!("{home}/.local/share/kwin/scripts/chrona-watcher");
    let copy = std::process::Command::new("cp")
        .arg("-rL")
        .arg(src)
        .arg(&dst)
        .output();
    match copy {
        Ok(o) if o.status.success() => {}
        Ok(o) => return format!("copy failed: {}", String::from_utf8_lossy(&o.stderr)),
        Err(e) => return format!("copy failed: {e}"),
    }

    let steps: [(&str, String); 4] = [
        (
            "install",
            format!("kpackagetool6 --type=KWin/Script -u {dst} 2>/dev/null || kpackagetool6 --type=KWin/Script -i {dst}"),
        ),
        (
            "enable",
            "kwriteconfig6 --file kwinrc --group Plugins --key chrona-watcherEnabled true".to_string(),
        ),
        (
            "start",
            "dbus-send --session --dest=org.kde.KWin /Scripting org.kde.kwin.Scripting.start".to_string(),
        ),
        ("reload", "dbus-send --session --dest=org.kde.KWin /KWin org.kde.KWin.reconfigure".to_string()),
    ];
    let mut report = String::new();
    for (name, cmd) in steps {
        match run_sh(&cmd) {
            Ok(_) => report.push_str(&format!("{name} ✓ ")),
            Err(e) => report.push_str(&format!("{name}: {e}; ")),
        }
    }
    format!("installed. {report}If it does not activate immediately, log out and back in.")
}

fn run_sh(cmd: &str) -> Result<(), String> {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn export_json() -> String {
    let Some(data) = client::request("export", json!({})) else {
        return "daemon offline".into();
    };
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let path = format!(
        "{}/chrona-export-{}.json",
        home,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    );
    let Ok(mut f) = std::fs::File::create(&path) else {
        return format!("cannot write {path}");
    };
    match serde_json::to_string_pretty(&data) {
        Ok(s) => {
            let _ = f.write_all(s.as_bytes());
            path
        }
        Err(e) => format!("export failed: {e}"),
    }
}
