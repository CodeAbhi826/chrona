//! App identity resolution: `.desktop` entries → pretty names + icon paths.
//!
//! Window watchers only hand us a machine id (WM_CLASS / Wayland app_id).
//! Everything a user actually recognises — "Firefox", "Steam", the real
//! icon — lives in freedesktop `.desktop` entries. This module scans the
//! standard application directories once (and re-scans when their mtime
//! changes), indexes every entry four ways and resolves the `Icon=` key
//! against the icon theme search path.
//!
//! Installed PWAs (Chrome/Brave/Chromium/Edge "Install app…") get desktop
//! files with `--app-id=<id>` in `Exec` and `StartupWMClass=crx_<id>`. We
//! index those under both keys, so a PWA window shows its own name and icon
//! instead of counting as plain browser time.
//!
//! Lookup priority per key (first writer wins): `StartupWMClass` > desktop
//! file stem > `Exec` basename > PWA app-id. All keys are lowercased — X11
//! WM_CLASS capitalisation ("Google-chrome") meets lowercase Wayland
//! app_ids ("google-chrome") halfway.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Minimum size we consider "good enough" when picking between icon hits.
const PREFERRED_SIZES: &[&str] = &[
    "512x512", "384x384", "256x256", "192x192", "128x128", "96x96", "72x72", "64x64", "48x48",
    "scalable", "32x32", "24x24", "22x22", "16x16",
];
const ICON_EXTS: &[&str] = &["svg", "png", "xpm"];
const RESCAN_INTERVAL: Duration = Duration::from_secs(600);

/// Resolved identity of one application.
#[derive(Debug, Clone)]
pub struct AppMeta {
    /// Human name from the desktop entry (e.g. "LibreOffice Writer").
    pub name: String,
    /// Absolute path of the best icon we found, if any.
    pub icon: Option<PathBuf>,
    /// True when the entry is an installed PWA (`--app-id=` in Exec).
    pub pwa: bool,
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    icon_raw: String,
    pwa: bool,
}

#[derive(Debug)]
pub struct AppIndex {
    by_key: HashMap<String, Entry>,
    scanned_at: SystemTime,
    dir_mtimes: Vec<(PathBuf, Option<SystemTime>)>,
    icon_roots: Vec<PathBuf>,
}

impl Default for AppIndex {
    fn default() -> Self {
        Self {
            by_key: HashMap::new(),
            scanned_at: SystemTime::UNIX_EPOCH,
            dir_mtimes: Vec::new(),
            icon_roots: Vec::new(),
        }
    }
}

impl AppIndex {
    /// Scan the freedesktop application + icon directories.
    pub fn scan_system() -> Self {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(PathBuf::from(&home).join(".local/share/applications"));
        }
        if let Ok(xdh) = std::env::var("XDG_DATA_HOME") {
            let d = PathBuf::from(xdh).join("applications");
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }
        let data_dirs =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        for p in std::env::split_paths(&data_dirs) {
            let d = p.join("applications");
            if !dirs.contains(&d) {
                dirs.push(d);
            }
        }

        let mut icon_roots: Vec<PathBuf> = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(&home);
            icon_roots.push(h.join(".icons"));
            icon_roots.push(h.join(".local/share/icons"));
        }
        if let Ok(dirs) = std::env::var("XDG_DATA_DIRS") {
            for p in std::env::split_paths(&dirs) {
                icon_roots.push(p.join("icons"));
            }
        }
        if let Ok(xdh) = std::env::var("XDG_DATA_HOME") {
            icon_roots.push(PathBuf::from(xdh).join("icons"));
        }
        icon_roots.push(PathBuf::from("/usr/share/icons"));
        icon_roots.push(PathBuf::from("/usr/share/pixmaps"));

        let mut idx = Self {
            icon_roots,
            ..Self::default()
        };
        idx.scan_dirs(&dirs);
        idx
    }

    /// Scan explicit directories (also the unit-test entry point).
    fn scan_dirs(&mut self, dirs: &[PathBuf]) {
        self.scanned_at = SystemTime::now();
        self.dir_mtimes = dirs.iter().map(|d| (d.clone(), dir_mtime(d))).collect();
        self.by_key.clear();
        for dir in dirs {
            let Ok(files) = std::fs::read_dir(dir) else {
                continue;
            };
            for f in files.flatten() {
                let path = f.path();
                if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                self.index_desktop_file(&path);
            }
        }
    }

    fn index_desktop_file(&mut self, path: &Path) {
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        // Only the [Desktop Entry] group matters; actions have their own
        // groups with duplicate keys which we must not read.
        let entry_text: String = text
            .lines()
            .take_while(|l| !l.trim_start().starts_with('[') || l.trim() == "[Desktop Entry]")
            .collect::<Vec<_>>()
            .join("\n");
        if !entry_text.contains("[Desktop Entry]") {
            return;
        }
        let get = |key: &str| -> Option<String> {
            entry_text
                .lines()
                .find_map(|l| {
                    let l = l.trim();
                    let rest = l.strip_prefix(key)?.trim_start();
                    rest.strip_prefix('=').map(|v| v.trim().to_string())
                })
                .filter(|v| !v.is_empty())
        };
        if get("Type").as_deref() != Some("Application") {
            return;
        }
        let Some(name) = get("Name") else { return };
        let icon_raw = get("Icon").unwrap_or_default();
        let exec = get("Exec").unwrap_or_default();
        let wm_class = get("StartupWMClass").unwrap_or_default();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let exec_base = exec
            .split_whitespace()
            .next()
            .map(|tok| {
                Path::new(tok)
                    .file_name()
                    .and_then(|f| f.to_str())
                    .unwrap_or(tok)
                    .to_string()
            })
            .unwrap_or_default();

        // PWA: Chrome-family desktop files carry --app-id=<id> in Exec.
        let app_id = exec
            .split_whitespace()
            .find_map(|tok| tok.strip_prefix("--app-id="))
            .map(|s| s.to_string());
        let pwa = app_id.is_some();
        let entry = Entry {
            name,
            icon_raw,
            pwa,
        };

        // Insertion order = priority: StartupWMClass, stem, exec, app-id.
        let mut keys: Vec<String> = Vec::new();
        if !wm_class.is_empty() {
            keys.push(wm_class.to_lowercase());
        }
        if !stem.is_empty() {
            keys.push(stem.to_lowercase());
        }
        if !exec_base.is_empty() {
            keys.push(exec_base.to_lowercase());
        }
        if let Some(id) = &app_id {
            keys.push(id.to_lowercase());
            keys.push(format!("crx_{}", id.to_lowercase()));
        }
        for k in keys {
            self.by_key.entry(k).or_insert(entry.clone());
        }
    }

    /// Look up an app by the id watchers report. Handles composite ids
    /// ("firefox:netflix" from demo seed data) by trying the host part.
    pub fn lookup(&self, app_id: &str) -> Option<AppMeta> {
        let key = app_id.trim().to_lowercase();
        let try_keys = |k: &str| {
            self.by_key.get(k).map(|e| AppMeta {
                name: e.name.clone(),
                icon: self.resolve_icon(&e.icon_raw),
                pwa: e.pwa,
            })
        };
        try_keys(&key).or_else(|| {
            // Reverse-DNS tail: "org.telegram.desktop" → "telegram"?
            // Only for ids with >= 3 dots, never the literal "desktop".
            if key.matches('.').count() >= 3 {
                key.rsplit('.').next().and_then(try_keys)
            } else {
                None
            }
        })
    }

    /// Resolve an `Icon=` value to an absolute file path. Values may be
    /// absolute paths, or theme icon names to find under the icon roots.
    fn resolve_icon(&self, raw: &str) -> Option<PathBuf> {
        if raw.is_empty() {
            return None;
        }
        let p = Path::new(raw);
        if p.is_absolute() {
            return p.is_file().then(|| p.to_path_buf());
        }
        if raw.contains('/') {
            return None; // relative path: not per-spec, don't guess
        }
        // Try the user's configured theme first (best visual match), then
        // hicolor (the guaranteed fallback theme), then any other theme,
        // then /usr/share/pixmaps.
        let mut theme_order: Vec<String> = Vec::new();
        if let Some(t) = read_icon_theme_setting() {
            theme_order.push(t);
        }
        theme_order.push("hicolor".into());
        let mut others: Vec<String> = Vec::new();
        for root in &self.icon_roots {
            if let Ok(rd) = std::fs::read_dir(root) {
                for e in rd.flatten() {
                    if let Some(n) = e.file_name().to_str() {
                        if e.path().is_dir() && !theme_order.contains(&n.to_string()) {
                            others.push(n.to_string());
                        }
                    }
                }
            }
        }
        theme_order.extend(others);

        for theme in &theme_order {
            for root in &self.icon_roots {
                for size in PREFERRED_SIZES {
                    for ext in ICON_EXTS {
                        let cand = root
                            .join(theme)
                            .join(size)
                            .join("apps")
                            .join(format!("{raw}.{ext}"));
                        if cand.is_file() {
                            return Some(cand);
                        }
                    }
                }
            }
        }
        // Legacy location: /usr/share/pixmaps/<name>.<ext>
        if let Some(root) = self.icon_roots.iter().find(|r| r.ends_with("pixmaps")) {
            for ext in ICON_EXTS {
                let cand = root.join(format!("{raw}.{ext}"));
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        None
    }

    /// Re-scan when a directory changed or the cache is older than ten
    /// minutes. Cheap when nothing changed (a handful of stat calls).
    pub fn refresh_if_stale(&mut self) {
        let stale = self.dir_mtimes.iter().any(|(d, m)| dir_mtime(d) != *m)
            || self
                .scanned_at
                .elapsed()
                .map(|e| e > RESCAN_INTERVAL)
                .unwrap_or(true);
        if stale {
            let dirs: Vec<PathBuf> = self.dir_mtimes.iter().map(|(d, _)| d.clone()).collect();
            if dirs.is_empty() {
                *self = Self::scan_system();
            } else {
                self.scan_dirs(&dirs);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.by_key.len()
    }
}

fn dir_mtime(d: &Path) -> Option<SystemTime> {
    std::fs::metadata(d).ok().and_then(|m| m.modified().ok())
}

/// Best-effort read of the current icon theme name from GNOME/KDE settings.
fn read_icon_theme_setting() -> Option<String> {
    // KDE: ~/.config/kdeglobals [Icons] Theme=…
    if let Ok(home) = std::env::var("HOME") {
        let kg = PathBuf::from(&home).join(".config/kdeglobals");
        if let Ok(text) = std::fs::read_to_string(kg) {
            let mut in_icons = false;
            for line in text.lines() {
                let l = line.trim();
                if l.starts_with('[') {
                    in_icons = l == "[Icons]";
                } else if in_icons && l.starts_with("Theme=") {
                    let t = l[6..].trim();
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
        // GNOME: ~/.config/gtk-3.0/settings.ini [Settings] gtk-icon-theme-name=…
        let ini = PathBuf::from(&home).join(".config/gtk-3.0/settings.ini");
        if let Ok(text) = std::fs::read_to_string(ini) {
            for line in text.lines() {
                let l = line.trim();
                if let Some(v) = l.strip_prefix("gtk-icon-theme-name=") {
                    let t = v.trim().trim_matches('"');
                    if !t.is_empty() {
                        return Some(t.to_string());
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base =
            std::env::temp_dir().join(format!("chrona-icons-test-{tag}-{}", std::process::id()));
        let apps = base.join("applications");
        let icons = base.join("icons/hicolor/48x48/apps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::create_dir_all(&icons).unwrap();
        (base, apps, icons)
    }

    fn write(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
    }

    fn build(dirs: &[PathBuf], icon_roots: Vec<PathBuf>) -> AppIndex {
        let mut idx = AppIndex {
            icon_roots,
            ..Default::default()
        };
        idx.scan_dirs(dirs);
        idx
    }

    #[test]
    fn resolves_name_wmclass_and_icon() {
        let (base, apps, icons) = fixture("basic");
        write(
            &apps.join("org.example.Zap.desktop"),
            "[Desktop Entry]\nType=Application\nName=Zap Editor\nExec=zap %F\nIcon=zap\nStartupWMClass=org.example.Zap\n",
        );
        write(&icons.join("zap.svg"), "<svg/>");
        let idx = build(
            std::slice::from_ref(&apps),
            vec![base.join("icons"), base.join("pixmaps")],
        );
        let m = idx.lookup("org.example.zap").expect("wmclass key");
        assert_eq!(m.name, "Zap Editor");
        assert!(m.icon.unwrap().ends_with("zap.svg"));
        let m2 = idx.lookup("Zap").expect("x11 wmclass is capitalised");
        assert_eq!(m2.name, "Zap Editor");
        assert!(idx.lookup("nope-missing").is_none());
    }

    #[test]
    fn pwa_entry_is_recognised_both_ways() {
        let (_base, apps, icons) = fixture("pwa");
        write(
            &apps.join("chrome-pwaapp-Default.desktop"),
            "[Desktop Entry]\nType=Application\nName=YouTube Music\nExec=/usr/bin/brave --app-id=abc123def456\nIcon=chrome-abc123def456-Default\nStartupWMClass=crx_abc123def456\n",
        );
        write(&icons.join("chrome-abc123def456-Default.png"), "png");
        let idx = build(std::slice::from_ref(&apps), vec![_base.join("icons")]);
        let m = idx.lookup("crx_abc123def456").expect("crx_ wmclass");
        assert_eq!(m.name, "YouTube Music");
        assert!(m.pwa);
        assert!(m.icon.unwrap().ends_with("chrome-abc123def456-Default.png"));
        let m2 = idx.lookup("abc123def456").expect("bare app-id key");
        assert!(m2.pwa);
    }

    #[test]
    fn icon_name_differs_from_id() {
        // vim.desktop: Icon=gvim although the desktop id is "vim".
        let (_base, apps, icons) = fixture("vim");
        write(
            &apps.join("vim.desktop"),
            "[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\nIcon=gvim\n",
        );
        write(&icons.join("gvim.png"), "png");
        let idx = build(std::slice::from_ref(&apps), vec![_base.join("icons")]);
        let m = idx.lookup("vim").expect("stem key");
        assert_eq!(m.name, "Vim");
        assert!(m.icon.unwrap().ends_with("gvim.png"));
    }

    #[test]
    fn ignores_non_application_and_action_groups() {
        let (_base, apps, _icons) = fixture("filter");
        write(
            &apps.join("x.desktop"),
            "[Desktop Entry]\nType=Application\nName=Good\nExec=good\nIcon=good\n\n[Desktop Action New]\nName=New Window\nExec=good --new\n",
        );
        write(
            &apps.join("link.desktop"),
            "[Desktop Entry]\nType=Link\nName=Bad\nURL=https://x\n",
        );
        let idx = build(std::slice::from_ref(&apps), vec![]);
        assert!(idx.lookup("good").is_some());
        assert!(
            idx.lookup("new window").is_none(),
            "action group keys ignored"
        );
        assert!(idx.lookup("bad").is_none(), "Type=Link skipped");
    }
}
