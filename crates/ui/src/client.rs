//! Daemon client + view-model building for the Chrona UI.

use crate::{
    AppItem, ArcItem, CatItem, ChronaApp, DayItem, GoalItem, HeatCell, HourItem, Theme, TitleItem,
    WeekColumn,
};
use chrono::Datelike;
use serde_json::{json, Value};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

pub static REFRESH: AtomicBool = AtomicBool::new(false);

/// Cached 30-day app list for the Apps view search filter.
///
/// Plain data (no `slint::Image` — it is not `Send`, so it cannot live in
/// a static); icons are loaded from `icon_path` when view models are built.
#[derive(Clone)]
struct AppSummary {
    app_id: String,
    name: String,
    seconds: i64,
    seconds_text: String,
    sessions: i64,
    share: f32,
    today_text: String,
    week_text: String,
    month_text: String,
    icon_path: Option<String>,
}

static APPS_CACHE: Mutex<Vec<AppSummary>> = Mutex::new(Vec::new());
static THEME_APPLIED: AtomicBool = AtomicBool::new(false);

/// App identity (name/icon/pwa) resolved by the daemon from .desktop
/// entries. Refreshed every tick for the ids currently displayed.
pub struct MetaInfo {
    pub name: String,
    pub icon: Option<String>,
    pub pwa: bool,
}
static META: Mutex<Vec<(String, MetaInfo)>> = Mutex::new(Vec::new());

fn meta_of(app_id: &str) -> Option<MetaInfo> {
    META.lock()
        .unwrap()
        .iter()
        .find(|(k, _)| k == app_id)
        .map(|(_, m)| MetaInfo {
            name: m.name.clone(),
            icon: m.icon.clone(),
            pwa: m.pwa,
        })
}

/// Ask the daemon to resolve a batch of app ids against .desktop entries.
fn fetch_meta(ids: &[String]) -> Vec<(String, MetaInfo)> {
    let mut out = Vec::new();
    if let Some(data) = request("apps.meta", json!({ "ids": ids })) {
        if let Some(obj) = data.as_object() {
            for (k, v) in obj {
                out.push((
                    k.clone(),
                    MetaInfo {
                        name: v
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        icon: v.get("icon").and_then(Value::as_str).map(str::to_string),
                        pwa: v.get("pwa").and_then(Value::as_bool).unwrap_or(false),
                    },
                ));
            }
        }
    }
    out
}

pub fn socket_path() -> PathBuf {
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| PathBuf::from(d).join("chrona.sock"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/chrona.sock"))
}

/// One JSON-RPC-ish round trip to the daemon. Returns `data` on ok.
/// Both directions carry timeouts: a wedged daemon (or a full socket
/// buffer) must never freeze the UI thread for longer than 5 s.
pub fn request(cmd: &str, args: Value) -> Option<Value> {
    let mut stream = UnixStream::connect(socket_path()).ok()?;
    let timeout = Some(Duration::from_secs(5));
    stream.set_read_timeout(timeout).ok()?;
    stream.set_write_timeout(timeout).ok()?;
    let req = json!({"id": 1, "cmd": cmd, "args": args});
    writeln!(stream, "{req}").ok()?;
    stream.flush().ok()?;
    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        v.get("data").cloned()
    } else {
        None
    }
}

pub fn fmt_dur(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        if m > 0 {
            format!("{h}h {m}m")
        } else {
            format!("{h}h")
        }
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{secs}s")
    }
}

/// "google-chrome" → "Google Chrome", "org.telegram.desktop" → "Telegram".
pub fn pretty_name(id: &str) -> String {
    let mut base = id;
    if base.contains('.') {
        let parts: Vec<&str> = base.split('.').collect();
        base = if parts.len() > 2
            && matches!(parts[0], "org" | "net" | "com" | "io" | "im")
            && !parts[1].is_empty()
        {
            parts[1]
        } else {
            parts[parts.len() - 1]
        };
    }
    base.replace(['-', '_'], " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sstr(s: impl Into<String>) -> SharedString {
    s.into().into()
}

/// Load an icon file into a Slint image (empty image on any failure —
/// the row then falls back to the colored dot).
fn load_icon(path: Option<&str>) -> slint::Image {
    path.and_then(|p| slint::Image::load_from_path(std::path::Path::new(p)).ok())
        .unwrap_or_default()
}

fn rc<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(v))
}

fn i64_of(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

/// Fraction of total per app, used for bars.
fn shares(apps: &[Value]) -> Vec<f32> {
    let max = apps.iter().map(|a| i64_of(a, "seconds")).max().unwrap_or(0);
    apps.iter()
        .map(|a| {
            if max > 0 {
                i64_of(a, "seconds") as f32 / max as f32
            } else {
                0.0
            }
        })
        .collect()
}

// ----- view-model builders ---------------------------------------------------

fn build_apps(payload: &Value) -> Vec<AppSummary> {
    let apps = arr(payload, "apps");
    let sh = shares(apps);
    apps.iter()
        .zip(sh)
        .map(|(a, share)| {
            let id = a.get("app_id").and_then(Value::as_str).unwrap_or("");
            let meta = meta_of(id);
            AppSummary {
                app_id: id.to_string(),
                name: meta
                    .as_ref()
                    .map(|m| m.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| pretty_name(id)),
                seconds: i64_of(a, "seconds"),
                seconds_text: fmt_dur(i64_of(a, "seconds")),
                sessions: i64_of(a, "sessions"),
                share,
                today_text: String::new(),
                week_text: String::new(),
                month_text: String::new(),
                icon_path: meta.and_then(|m| m.icon),
            }
        })
        .collect()
}

/// AppSummary → the struct the Slint views consume.
fn to_item(s: &AppSummary) -> AppItem {
    AppItem {
        app_id: sstr(s.app_id.clone()),
        name: sstr(s.name.clone()),
        seconds: s.seconds as i32,
        seconds_text: sstr(s.seconds_text.clone()),
        sessions: s.sessions as i32,
        share: s.share,
        today_text: sstr(s.today_text.clone()),
        week_text: sstr(s.week_text.clone()),
        month_text: sstr(s.month_text.clone()),
        icon: load_icon(s.icon_path.as_deref()),
        has_icon: s.icon_path.is_some(),
        pwa: false,
    }
}

/// SVG arc command on a 100×100 viewbox, radius 45, clockwise from top.
/// `sweep_deg` is capped below 360 (a full circle cannot be one arc).
pub fn arc_cmd(start_deg: f32, sweep_deg: f32) -> String {
    let sweep = sweep_deg.clamp(0.0, 359.9);
    if sweep <= 0.05 {
        return String::new();
    }
    let (x0, y0) = ring_point(start_deg);
    let (x1, y1) = ring_point(start_deg + sweep);
    let large = if sweep > 180.0 { 1 } else { 0 };
    format!("M {x0:.2} {y0:.2} A 45 45 0 {large} 1 {x1:.2} {y1:.2}")
}

fn ring_point(deg: f32) -> (f32, f32) {
    let rad = deg.to_radians();
    (50.0 + 45.0 * rad.sin(), 50.0 - 45.0 * rad.cos())
}

/// Donut + stacked-bar geometry per category list.
fn build_cats(cats: &[Value], range_max: i64) -> Vec<CatItem> {
    let total: i64 = cats.iter().map(|c| i64_of(c, "seconds")).sum();
    let mut y0 = 0f32;
    cats.iter()
        .map(|c| {
            let secs = i64_of(c, "seconds");
            let share = if total > 0 {
                secs as f32 / total as f32
            } else {
                0.0
            };
            let v = if range_max > 0 {
                secs as f32 / range_max as f32
            } else {
                0.0
            };
            let seg = CatItem {
                key: sstr(
                    c.get("key")
                        .and_then(Value::as_str)
                        .unwrap_or("uncategorised"),
                ),
                label: sstr(
                    c.get("label")
                        .and_then(Value::as_str)
                        .unwrap_or("Uncategorised"),
                ),
                seconds: secs as i32,
                seconds_text: sstr(fmt_dur(secs)),
                share,
                pct_text: sstr(format!("{}%", (share * 100.0).round() as i32)),
                v,
                y0,
            };
            y0 += v;
            seg
        })
        .collect()
}

/// Donut arcs from a category payload.
fn build_arcs(cats: &[Value]) -> Vec<ArcItem> {
    let total: i64 = cats.iter().map(|c| i64_of(c, "seconds")).sum();
    let mut start = 0f32;
    cats.iter()
        .map(|c| {
            let share = if total > 0 {
                i64_of(c, "seconds") as f32 / total as f32
            } else {
                0.0
            };
            let cmd = arc_cmd(start, share * 360.0);
            start += share * 360.0;
            ArcItem {
                cmd: sstr(cmd),
                key: sstr(
                    c.get("key")
                        .and_then(Value::as_str)
                        .unwrap_or("uncategorised"),
                ),
            }
        })
        .collect()
}

fn build_hourly(hourly: &[Value]) -> Vec<HourItem> {
    let max = hourly.iter().filter_map(Value::as_i64).max().unwrap_or(0);
    hourly
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let val = v.as_i64().unwrap_or(0);
            HourItem {
                v: if max > 0 {
                    val as f32 / max as f32
                } else {
                    0.0
                },
                label: sstr(if i % 3 == 0 {
                    format!("{i:02}")
                } else {
                    String::new()
                }),
            }
        })
        .collect()
}

fn build_days(days: &[Value]) -> (Vec<DayItem>, i64) {
    let max = days.iter().map(|d| i64_of(d, "seconds")).max().unwrap_or(0);
    let items = days
        .iter()
        .map(|d| {
            let secs = i64_of(d, "seconds");
            let date = d.get("date").and_then(Value::as_str).unwrap_or("");
            DayItem {
                date: sstr(date),
                label: sstr(short_date(date)),
                seconds: secs as i32,
                seconds_text: sstr(fmt_dur(secs)),
                v: if max > 0 {
                    secs as f32 / max as f32
                } else {
                    0.0
                },
                cats: rc(build_cats(arr(d, "categories"), max)),
            }
        })
        .collect();
    (items, max)
}

/// "2026-08-29" → "Sat 29"
fn short_date(date: &str) -> String {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|d| format!("{}", d.format("%a %d")))
        .unwrap_or_else(|| date.to_string())
}

fn weekday_of(date: &str) -> u32 {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|d| d.weekday().num_days_from_monday())
        .unwrap_or(0)
        + 1 // 1..=7, Monday..=Sunday
}

/// Month heatmap: weeks as columns, cells Mon..Sun.
fn build_heatmap(days: &[Value]) -> Vec<WeekColumn> {
    use std::collections::BTreeMap;
    let mut by_date: BTreeMap<String, f32> = BTreeMap::new();
    let max = days.iter().map(|d| i64_of(d, "seconds")).max().unwrap_or(0);
    for d in days {
        let date = d
            .get("date")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !date.is_empty() {
            by_date.insert(
                date.clone(),
                if max > 0 {
                    i64_of(d, "seconds") as f32 / max as f32
                } else {
                    0.0
                },
            );
        }
    }
    let today_label = chrono::Local::now().format("%Y-%m-%d").to_string();

    let mut weeks = Vec::new();
    let mut column: Vec<HeatCell> = Vec::new();
    if let Some(first) = by_date.keys().next() {
        for _ in 1..weekday_of(first) {
            column.push(HeatCell {
                v: -1.0,
                label: sstr(""),
                tip: sstr(""),
                is_today: false,
            });
        }
    }
    for (date, v) in &by_date {
        column.push(HeatCell {
            v: *v,
            label: sstr(date),
            tip: sstr(format!(
                "{date}: {} of screen time",
                fmt_dur((*v * max as f32) as i64)
            )),
            is_today: &today_label == date,
        });
        if column.len() == 7 {
            weeks.push(WeekColumn {
                cells: rc(std::mem::take(&mut column)),
            });
        }
    }
    if !column.is_empty() {
        weeks.push(WeekColumn { cells: rc(column) });
    }
    weeks
}

// ----- the polling tick --------------------------------------------------------
//
// v0.2.0 ran this from a `std::thread` and called `weak.upgrade()` there.
// slint::Weak::upgrade() only works on the thread that created the component
// (returns None elsewhere), so the UI never saw the daemon and showed
// "Chrona daemon is not running" even with a healthy chronad. Polling now
// happens from two slint::Timer callbacks, which run on the event-loop thread
// where the upgrade is valid.

pub fn tick(app: &ChronaApp) {
    let status = match request("status", json!({})) {
        Some(s) => s,
        None => {
            app.set_daemon_online(false);
            return;
        }
    };
    app.set_daemon_online(true);
    app.set_status_watcher(sstr(
        status.get("watcher").and_then(Value::as_str).unwrap_or(""),
    ));
    app.set_idle_line(sstr(
        status
            .get("idle_provider")
            .and_then(Value::as_str)
            .unwrap_or(""),
    ));
    app.set_version_line(sstr(
        status.get("version").and_then(Value::as_str).unwrap_or("?"),
    ));
    app.set_db_path(sstr(
        status.get("db_path").and_then(Value::as_str).unwrap_or("?"),
    ));
    app.set_tracking_paused(
        status
            .get("paused")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    );
    if let Some(cw) = status.get("current_window") {
        let w = cw.get("title").and_then(Value::as_str).unwrap_or("");
        let a = cw.get("app_id").and_then(Value::as_str).unwrap_or("");
        app.set_status_current(sstr(if !w.is_empty() {
            format!("{} — {}", pretty_name(a), w)
        } else {
            String::new()
        }));
    } else {
        app.set_status_current(sstr(""));
    }

    // Apply persisted theme once at startup.
    if !THEME_APPLIED.swap(true, Ordering::SeqCst) {
        if let Some(v) =
            request("settings.get", json!({"key": "theme"})).and_then(|d| d.get("value").cloned())
        {
            let dark = v.as_str() == Some("dark");
            app.global::<Theme>().set_dark(dark);
        }
    }

    // ---- shared payloads ----
    let day = request("day", json!({}));
    let week = request("week", json!({}));
    let month = request("month", json!({}));

    // ---- app identity (names/icons from .desktop entries) ----
    {
        let mut ids: Vec<String> = Vec::new();
        for p in [&day, &week, &month].into_iter().flatten() {
            for a in arr(p, "apps") {
                if let Some(id) = a.get("app_id").and_then(Value::as_str) {
                    if !ids.iter().any(|x| x == id) {
                        ids.push(id.to_string());
                    }
                }
            }
        }
        *META.lock().unwrap() = fetch_meta(&ids);
    }

    // ---- today ----
    if let Some(day) = &day {
        let total = i64_of(day, "total_seconds");
        app.set_today_date_text(sstr(chrono::Local::now().format("%A, %d %B").to_string()));
        app.set_today_total_text(sstr(fmt_dur(total)));
        app.set_today_unlocks(sstr(i64_of(day, "unlocks").to_string()));
        app.set_today_longest(sstr(fmt_dur(i64_of(day, "longest_session"))));
        app.set_today_apps(rc(build_apps(day).iter().map(to_item).take(6).collect()));
        let cats = arr(day, "categories").to_vec();
        app.set_today_cats(rc(build_cats(&cats, 0)));
        app.set_today_cat_arcs(rc(build_arcs(&cats)));
        let hourly = arr(day, "hourly").to_vec();
        app.set_today_hourly(rc(build_hourly(&hourly)));

        // Ring: today vs. the "usual day" (prev-week daily average), else 8h.
        if let Some(week) = &week {
            let prev = i64_of(week, "prev_total_seconds");
            let ratio = if prev > 0 {
                let usual = (prev / 7).max(1);
                app.set_today_ring_caption(sstr(format!("usual: {}", fmt_dur(usual))));
                total as f32 / usual as f32
            } else {
                app.set_today_ring_caption(sstr("of 8h budget"));
                total as f32 / (8 * 3600) as f32
            };
            app.set_today_ring_cmd(sstr(arc_cmd(0.0, ratio.clamp(0.0, 1.0) * 360.0)));
        }
    }

    // ---- week ----
    if let Some(week) = &week {
        let total = i64_of(week, "total_seconds");
        let prev = i64_of(week, "prev_total_seconds");
        app.set_week_total_text(sstr(fmt_dur(total)));
        app.set_week_avg_text(sstr(format!("{} average per day", fmt_dur(total / 7))));
        let delta = total - prev;
        app.set_week_delta_up(delta > 0);
        app.set_week_delta_text(sstr(if prev > 0 {
            format!(
                "{} vs last week ({})",
                if delta >= 0 { "+" } else { "" },
                fmt_dur(delta.abs())
            )
        } else {
            "first tracked week".to_string()
        }));
        app.set_week_apps(rc(build_apps(week).iter().map(to_item).take(8).collect()));
        let days = arr(week, "days").to_vec();
        let (day_items, _) = build_days(&days);
        app.set_week_days(rc(day_items));
    }

    // ---- month ----
    if let Some(month) = &month {
        app.set_month_label(sstr(
            month
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or("This month"),
        ));
        let total = i64_of(month, "total_seconds");
        let days = arr(month, "days").to_vec();
        let tracked = days
            .iter()
            .filter(|d| i64_of(d, "seconds") > 0)
            .count()
            .max(1) as i64;
        app.set_month_total_text(sstr(fmt_dur(total)));
        app.set_month_avg_text(sstr(format!(
            "{} average per active day",
            fmt_dur(total / tracked)
        )));
        let busiest = days.iter().max_by_key(|d| i64_of(d, "seconds"));
        app.set_month_busiest_text(sstr(
            busiest
                .map(|d| {
                    format!(
                        "busiest day: {} ({})",
                        d.get("date")
                            .and_then(Value::as_str)
                            .map(short_date)
                            .unwrap_or_default(),
                        fmt_dur(i64_of(d, "seconds"))
                    )
                })
                .unwrap_or_else(|| "no data yet".into()),
        ));
        app.set_month_weeks(rc(build_heatmap(&days)));
        app.set_month_apps(rc(build_apps(month).iter().map(to_item).take(8).collect()));
    }

    // ---- apps (30-day table) + per-range columns ----
    if let (Some(day), Some(week), Some(month)) = (&day, &week, &month) {
        let today_map: std::collections::HashMap<String, String> = arr(day, "apps")
            .iter()
            .map(|a| {
                (
                    a.get("app_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    fmt_dur(i64_of(a, "seconds")),
                )
            })
            .collect();
        let week_map: std::collections::HashMap<String, String> = arr(week, "apps")
            .iter()
            .map(|a| {
                (
                    a.get("app_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    fmt_dur(i64_of(a, "seconds")),
                )
            })
            .collect();
        let month_map: std::collections::HashMap<String, String> = arr(month, "apps")
            .iter()
            .map(|a| {
                (
                    a.get("app_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    fmt_dur(i64_of(a, "seconds")),
                )
            })
            .collect();
        let mut list = build_apps(month);
        for item in list.iter_mut() {
            let id = item.app_id.clone();
            item.today_text = today_map.get(&id).cloned().unwrap_or_else(|| "—".into());
            item.week_text = week_map.get(&id).cloned().unwrap_or_else(|| "—".into());
            item.month_text = month_map
                .get(&id)
                .cloned()
                .unwrap_or_else(|| fmt_dur(item.seconds));
        }
        list.sort_by_key(|a| std::cmp::Reverse(a.seconds));
        *APPS_CACHE.lock().unwrap() = list.clone();
        apply_search(app, &app.get_apps_search());
    }

    // ---- goals (data is a bare JSON array) ----
    if let Some(goals) = request("goals", json!({})) {
        let mut v: Vec<GoalItem> = goals
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|g| {
                let kind = g.get("kind").and_then(Value::as_str).unwrap_or("app");
                let key = g.get("key").and_then(Value::as_str).unwrap_or("");
                let used = i64_of(g, "used_seconds");
                let limit = i64_of(g, "limit_seconds");
                GoalItem {
                    id: i64_of(g, "id") as i32,
                    kind: sstr(kind),
                    key: sstr(key),
                    label: sstr(if kind == "total" {
                        "Screen time".into()
                    } else if kind == "category" {
                        chrona_label(key)
                    } else {
                        pretty_name(key)
                    }),
                    limit_text: sstr(fmt_dur(limit)),
                    used_text: sstr(fmt_dur(used)),
                    progress: if limit > 0 {
                        (used as f32 / limit as f32).min(1.0)
                    } else {
                        0.0
                    },
                    enabled: g.get("enabled").and_then(Value::as_bool).unwrap_or(true),
                    exceeded: used > limit && limit > 0,
                }
            })
            .collect::<Vec<_>>();
        // Screen-time (total) goal first — it is the headline limit.
        v.sort_by_key(|g| if g.kind.as_str() == "total" { 0 } else { 1 });
        app.set_goals(rc(v));
        // Fill suggestions once; afterwards only the kind-changed callback
        // touches them, so we never reset the user's ComboBox mid-edit.
        if app.get_goal_suggestions().row_count() == 0 {
            apply_goal_suggestions(app, "category");
        }
    }
}

fn chrona_label(key: &str) -> String {
    // mirrors chrona-core Category::display
    match key {
        "work" => "Work & Coding".into(),
        "browsers" => "Browsers".into(),
        "communication" => "Communication".into(),
        "media" => "Media & Streaming".into(),
        "creative" => "Creative & Design".into(),
        "gaming" => "Games".into(),
        "system" => "System & Files".into(),
        _ => "Uncategorised".into(),
    }
}

pub fn apply_goal_suggestions(app: &ChronaApp, kind: &str) {
    let list: Vec<SharedString> = if kind == "total" {
        vec![sstr("total")]
    } else if kind == "category" {
        [
            "work",
            "browsers",
            "communication",
            "media",
            "creative",
            "gaming",
            "system",
        ]
        .iter()
        .map(|k| sstr(*k))
        .collect()
    } else {
        APPS_CACHE
            .lock()
            .unwrap()
            .iter()
            .take(12)
            .map(|a| sstr(a.app_id.to_string()))
            .collect()
    };
    app.set_goal_suggestions(rc(list));
}

pub fn apply_search(app: &ChronaApp, query: &str) {
    let q = query.to_lowercase();
    let filtered: Vec<AppItem> = APPS_CACHE
        .lock()
        .unwrap()
        .iter()
        .filter(|a| {
            q.is_empty()
                || a.name.to_lowercase().contains(&q)
                || a.app_id.to_lowercase().contains(&q)
        })
        .map(to_item)
        .collect();
    app.set_apps_list(rc(filtered));
}

/// Load app detail (titles) into the UI. Called from the select-app callback.
pub fn load_app_detail(app: &ChronaApp, app_id: &str) {
    let Some(data) = request("app", json!({"app_id": app_id, "days": 7})) else {
        return;
    };
    let titles = arr(&data, "titles");
    let total: i64 = titles.iter().map(|t| i64_of(t, "seconds")).sum();
    let sessions: i64 = titles.iter().map(|t| i64_of(t, "sessions")).sum();
    app.set_app_detail_name(sstr(
        meta_of(app_id)
            .map(|m| m.name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| pretty_name(app_id)),
    ));
    app.set_app_detail_total(sstr(fmt_dur(total)));
    app.set_app_detail_sessions(sstr(sessions.to_string()));
    app.set_app_titles(rc(titles
        .iter()
        .map(|t| TitleItem {
            title: sstr(t.get("title").and_then(Value::as_str).unwrap_or("")),
            seconds_text: sstr(fmt_dur(i64_of(t, "seconds"))),
            sessions_text: sstr(format!("{} sessions", i64_of(t, "sessions"))),
        })
        .collect()));
}
