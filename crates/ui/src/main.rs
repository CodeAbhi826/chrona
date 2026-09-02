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
    // The file is written once and only rewritten when the bundled font
    // actually changed (size differs) — no per-launch 300 KB rewrite, and
    // a newer build updates the cache automatically.
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let cache = std::env::var("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(&home).join(".cache"));
    let dir = cache.join("chrona");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("Inter.ttf");
    let stale = std::fs::metadata(&path)
        .map(|m| m.len() != INTER_FONT.len() as u64)
        .unwrap_or(true);
    if stale {
        // Best-effort refresh; a read-only cache dir just keeps the old font.
        let _ = std::fs::write(&path, INTER_FONT);
    }
    if path.exists() {
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
        app.on_toggle_goal(move |id, _on| {
            // Single atomic round trip on the daemon — the old flow (read
            // `goals`, then `goal.set` the row back with a flipped flag)
            // raced against other clients and dropped concurrent edits.
            let _ = client::request("goal.toggle", json!({"id": id}));
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
        app.on_install_gnome(move || {
            let status = install_gnome_extension();
            if let Some(app) = weak.upgrade() {
                app.set_gnome_status(status.into());
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

/// Register + enable the KWin watcher script for the current user.
/// Tries, in order: packaged script at /usr/share/chrona, the repo
/// checkout (cargo run), then gives up with a helpful message.
///
/// kpackagetool is pointed at the package SOURCE, never at the installed
/// copy — `--upgrade` uninstalls (deletes) the installed package first, so
/// upgrading "from" the install location fails with "No such file". No
/// manual copy is done at all: kpackagetool installs into
/// ~/.local/share/kwin/scripts by itself.
///
/// All helper processes are invoked with explicit argv — never through a
/// shell — so a hostile `src` path can never inject commands.
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

    let pick = |names: &[&str]| {
        names
            .iter()
            .find(|n| {
                std::process::Command::new(n)
                    .arg("--version")
                    .output()
                    .is_ok()
            })
            .map(|s| s.to_string())
    };
    let Some(kpt) = pick(&["kpackagetool6", "kpackagetool5"]) else {
        return "kpackagetool not found — install Plasma's kpackage tools".into();
    };
    let kwc = pick(&["kwriteconfig6", "kwriteconfig5"]).unwrap_or_else(|| "kwriteconfig6".into());

    // Install or upgrade from the SOURCE dir. The list check tells us which.
    let installed = std::process::Command::new(&kpt)
        .args(["--type=KWin/Script", "--list"])
        .output()
        .map(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).contains("chrona-watcher")
        })
        .unwrap_or(false);
    let verb = if installed { "-u" } else { "-i" };
    let out = std::process::Command::new(&kpt)
        .args(["--type=KWin/Script", verb])
        .arg(src)
        .output();
    if let Err(e) = out {
        return format!("kpackagetool could not be run: {e}");
    }
    if !out.unwrap().status.success() {
        return "kpackagetool failed — see the journal for details".into();
    }

    // Enable in kwinrc ([Plugins] per the KDE docs, plus [Scripts] for older
    // Plasma 5 layouts); ask KWin to reload. All best-effort.
    let _ = std::process::Command::new(&kwc)
        .args([
            "--file",
            "kwinrc",
            "--group",
            "Plugins",
            "--key",
            "chrona-watcherEnabled",
            "true",
        ])
        .output();
    let _ = std::process::Command::new(&kwc)
        .args([
            "--file",
            "kwinrc",
            "--group",
            "Scripts",
            "--key",
            "chrona-watcherEnabled",
            "true",
        ])
        .output();
    let _ = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.kde.KWin",
            "/Scripting",
            "org.kde.kwin.Scripting.start",
        ])
        .output();
    let _ = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--dest=org.kde.KWin",
            "/KWin",
            "org.kde.KWin.reconfigure",
        ])
        .output();

    "installed and enabled. If it does not activate immediately, log out and back in.".into()
}

/// Register + enable the Chrona GNOME Shell extension for the current user.
/// Tries, in order: packaged copy at /usr/share/chrona/gnome, the repo
/// checkout (cargo run), then gives up with a helpful message.
///
/// The extension dir is copied to
/// ~/.local/share/gnome-shell/extensions/chrona@chrona.local — GNOME only
/// scans that directory at shell startup, so enabling is best-effort: if
/// `gnome-extensions enable` cannot see it yet, the user logs out/in once.
fn install_gnome_extension() -> String {
    const UUID: &str = "chrona@chrona.local";
    let candidates = [
        std::path::PathBuf::from("/usr/share/chrona/gnome").join(UUID),
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../gnome")
            .join(UUID)
            .canonicalize()
            .unwrap_or_else(|_| std::path::PathBuf::from("/nonexistent")),
    ];
    let Some(src) = candidates.iter().find(|p| p.exists()) else {
        return "GNOME extension not found — install the chrona package or run from the repo"
            .into();
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dest = std::path::PathBuf::from(&home)
        .join(".local/share/gnome-shell/extensions")
        .join(UUID);
    // Replace any older copy (remove first — a plain copy would leave
    // stale files behind).
    let _ = std::fs::remove_dir_all(&dest);
    if std::fs::create_dir_all(dest.parent().unwrap_or(&dest)).is_err() {
        return "could not create the extensions directory".into();
    }
    if let Err(e) = copy_dir(src, &dest) {
        return format!("copying the extension failed: {e}");
    }

    // Enable it (works only when the shell already knows the extension —
    // i.e. it was loaded at login). Best-effort, plus a re-scan attempt.
    let _ = std::process::Command::new("gnome-extensions")
        .arg("enable")
        .arg(UUID)
        .output();
    if !dest.join("extension.js").exists() {
        return "extension files incomplete — re-install the chrona package".into();
    }
    "installed. Log out and back in once, then it tracks windows automatically.".into()
}

/// Recursive directory copy (`cp -r` without a shell).
fn copy_dir(src: &std::path::Path, dest: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
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
