//! Local JSON API over a Unix socket (one JSON request per line, one JSON
//! response per line). Used by the Chrona UI and available for scripting
//! (`socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/chrona.sock`).

use crate::icons::AppIndex;
use crate::state::Tracker;
use chrona_core::{rules::RuleSet, stats};
use chrona_store::{Goal, Store};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Shared {
    pub store: Store,
    pub tracker: Mutex<Tracker>,
    pub started: std::time::Instant,
    pub watcher: Mutex<String>,
    pub idle_provider: Mutex<String>,
    pub ruleset: Mutex<Arc<RuleSet>>,
    /// User-level recording pause (ActivityWatch-style). Read by the state
    /// machine loop, set through the `pause.set` API command.
    pub paused: Arc<AtomicBool>,
    /// .desktop entry index: pretty names, real icons, PWA detection.
    pub icons: Mutex<AppIndex>,
}

impl Shared {
    pub fn new(store: Store) -> anyhow::Result<Self> {
        // `compile` is infallible: invalid user patterns degrade to literal
        // matches instead of disabling the whole ruleset.
        let ruleset = Arc::new(RuleSet::compile(&store.rules()?));
        let paused = Arc::new(AtomicBool::new(
            store.setting("paused").ok().flatten().as_deref() == Some("1"),
        ));
        let icons = AppIndex::scan_system();
        eprintln!(
            "[chronad] app index: {} entries from .desktop files",
            icons.len()
        );
        Ok(Self {
            store,
            tracker: Mutex::new(Tracker::new()),
            started: std::time::Instant::now(),
            watcher: Mutex::new("detecting…".into()),
            idle_provider: Mutex::new("detecting…".into()),
            ruleset: Mutex::new(ruleset),
            paused,
            icons: Mutex::new(icons),
        })
    }

    pub fn ruleset(&self) -> Arc<RuleSet> {
        Arc::clone(&self.ruleset.lock().unwrap())
    }

    pub fn rebuild_ruleset(&self) {
        let rules = self.store.rules().unwrap_or_default();
        let rs = Arc::new(RuleSet::compile(&rules));
        *self.ruleset.lock().unwrap() = rs;
    }

    pub fn set_watcher_label(&self, s: &str) {
        *self.watcher.lock().unwrap() = s.to_string();
    }

    pub fn set_idle_label(&self, s: &str) {
        *self.idle_provider.lock().unwrap() = s.to_string();
    }
}

pub fn default_socket_path() -> PathBuf {
    let run_dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    run_dir.join("chrona.sock")
}

pub fn serve(path: PathBuf, shared: Arc<Shared>) -> anyhow::Result<()> {
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    // Only the same user may talk to us.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let sh = Arc::clone(&shared);
        std::thread::spawn(move || {
            let _ = handle_conn(stream, sh);
        });
    }
    Ok(())
}

fn handle_conn(stream: UnixStream, shared: Arc<Shared>) -> anyhow::Result<()> {
    let mut writer = stream.try_clone()?;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(v) => dispatch(&shared, &v),
            Err(e) => json!({"id": 0, "ok": false, "error": e.to_string()}),
        };
        writeln!(writer, "{resp}")?;
        writer.flush()?;
    }
    Ok(())
}

fn dispatch(shared: &Shared, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(json!(0));
    let cmd = req.get("cmd").and_then(Value::as_str).unwrap_or("");
    let args = req.get("args").cloned().unwrap_or(json!({}));
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| handle_cmd(shared, cmd, &args)));
    match result {
        Ok(Ok(data)) => json!({"id": id, "ok": true, "data": data}),
        Ok(Err(e)) => json!({"id": id, "ok": false, "error": e.to_string()}),
        Err(panic) => {
            // A panic payload is almost always a String or &str; surface it
            // so failures are debuggable instead of a bare "internal error".
            let detail = panic
                .downcast_ref::<String>()
                .map(|s| s.as_str())
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .unwrap_or("unknown panic");
            json!({"id": id, "ok": false, "error": format!("internal error: {detail}")})
        }
    }
}

// ----- date helpers ---------------------------------------------------------

fn day_bounds(d: NaiveDate) -> (i64, i64) {
    let from = d
        .and_hms_opt(0, 0, 0)
        .and_then(|n| Local.from_local_datetime(&n).single())
        .map(|x| x.timestamp())
        .unwrap_or(0);
    let to = (d + Duration::days(1))
        .and_hms_opt(0, 0, 0)
        .and_then(|n| Local.from_local_datetime(&n).single())
        .map(|x| x.timestamp())
        .unwrap_or(i64::MAX);
    (from, to)
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn monday_of_week(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

// ----- command handlers ------------------------------------------------------

fn handle_cmd(sh: &Shared, cmd: &str, a: &Value) -> anyhow::Result<Value> {
    match cmd {
        // ----- introspection -----
        "ping" => Ok(json!({"pong": true})),
        "status" => {
            let t = sh.tracker.lock().unwrap();
            Ok(json!({
                "version": env!("CARGO_PKG_VERSION"),
                "uptime_seconds": sh.started.elapsed().as_secs(),
                "watcher": *sh.watcher.lock().unwrap(),
                "idle_provider": *sh.idle_provider.lock().unwrap(),
                "recording": !t.is_afk(),
                "afk": t.is_afk(),
                "current_window": t.current_window().map(|(app, title)| json!({"app_id": app, "title": title})),
                "db_path": sh.store.path().display().to_string(),
                "socket": default_socket_path().display().to_string(),
                "rules": sh.ruleset().len(),
                "paused": sh.paused.load(Ordering::Relaxed),
            }))
        }

        // ----- recording pause (ActivityWatch-style privacy switch) -----
        "pause.set" => {
            let p = a
                .get("paused")
                .and_then(Value::as_bool)
                .ok_or_else(|| anyhow::anyhow!("paused (bool) required"))?;
            sh.paused.store(p, Ordering::Relaxed);
            sh.store.set_setting("paused", if p { "1" } else { "0" })?;
            Ok(json!({"paused": p}))
        }

        // ----- queries -----
        "apps.meta" => {
            // Resolve app ids to {name, icon, pwa} from .desktop entries.
            // Refresh the index first so a freshly installed app shows up.
            {
                let mut idx = sh.icons.lock().unwrap();
                idx.refresh_if_stale();
            }
            let idx = sh.icons.lock().unwrap();
            let ids: Vec<String> = a
                .get("ids")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(Value::as_str)
                        .take(256)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mut out = serde_json::Map::new();
            for id in ids {
                if let Some(m) = idx.lookup(&id) {
                    out.insert(
                        id,
                        json!({
                            "name": m.name,
                            "icon": m.icon.as_ref().map(|p| p.display().to_string()),
                            "pwa": m.pwa,
                        }),
                    );
                }
            }
            Ok(json!(out))
        }
        "day" => {
            let d = a
                .get("date")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .unwrap_or_else(|| Local::now().date_naive());
            let (from, to) = day_bounds(d);
            let mut p = range_payload(sh, from, to, 0);
            p["date"] = json!(d.format("%Y-%m-%d").to_string());
            p["timeline"] = timeline_payload(sh, from, to);
            Ok(p)
        }
        "week" => {
            // Sanity-clamped: weeks in the past (0 .. ~10 years).
            let offset = a
                .get("offset")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, 520);
            let today = Local::now().date_naive();
            let monday = monday_of_week(today) - Duration::weeks(offset);
            let sunday = monday + Duration::days(6);
            let (from, to) = (day_bounds(monday).0, day_bounds(sunday).1);
            let mut p = range_payload(sh, from, to, 0);
            p["days"] = days_payload(sh, from, to);
            p["prev_total_seconds"] = json!(range_totals(sh, from - 7 * 86_400, from));
            Ok(p)
        }
        "month" => {
            // Sanity-clamped: months in the past (0 .. ~200 years).
            let offset = a
                .get("offset")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                .clamp(0, 2400) as i32;
            let now = Local::now().date_naive();
            let (y, m) = month_shift(now.year(), now.month() as i32 - offset);
            let first = NaiveDate::from_ymd_opt(y, m as u32, 1).unwrap();
            let last = month_shift(y, m + 1);
            let last_date =
                NaiveDate::from_ymd_opt(last.0, last.1 as u32, 1).unwrap() - Duration::days(1);
            let (from, to) = (day_bounds(first).0, day_bounds(last_date).1);
            let mut p = range_payload(sh, from, to, 0);
            p["days"] = days_payload(sh, from, to);
            p["label"] = json!(first.format("%B %Y").to_string());
            Ok(p)
        }
        "range" => {
            let from_d = a
                .get("from")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .ok_or_else(|| anyhow::anyhow!("from (YYYY-MM-DD) required"))?;
            let to_d = a
                .get("to")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .unwrap_or_else(|| from_d + Duration::days(1));
            let mut p = range_payload(sh, day_bounds(from_d).0, day_bounds(to_d).1, 0);
            p["days"] = days_payload(sh, day_bounds(from_d).0, day_bounds(to_d).1);
            Ok(p)
        }
        "app" => {
            let app_id = a
                .get("app_id")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("app_id required"))?
                .to_string();
            let days = a
                .get("days")
                .and_then(Value::as_i64)
                .unwrap_or(7)
                .clamp(1, 90);
            let now = Local::now();
            let (from, to) = (
                day_bounds(now.date_naive() - Duration::days(days - 1)).0,
                now.timestamp() + 60,
            );
            let events = sh.store.events_range(from, to)?;
            let afk = sh.store.afk_range(from, to)?;
            let active = stats::subtract_afk(&events, &afk);
            let rules = sh.ruleset();
            let titles: Vec<Value> = stats::titles_for(&active, &app_id)
                .into_iter()
                .take(15)
                .map(|t| json!({"title": t.title, "seconds": t.seconds, "sessions": t.sessions}))
                .collect();
            let daily = stats::daily_usage(
                &active
                    .into_iter()
                    .filter(|e| e.app_id == app_id)
                    .collect::<Vec<_>>(),
                from,
                to,
                &rules,
            );
            Ok(json!({
                "app_id": app_id,
                "days": days,
                "titles": titles,
                "daily": daily.iter().map(|d| json!({"date": d.date, "seconds": d.seconds})).collect::<Vec<_>>(),
            }))
        }

        // ----- rules -----
        "rules" => {
            let rules = sh.store.rules()?;
            Ok(json!(rules
                .iter()
                .map(|r| json!({
                    "id": r.id, "pattern": r.pattern, "field": r.field.key(),
                    "category": r.category.key(), "priority": r.priority,
                }))
                .collect::<Vec<_>>()))
        }
        "rule.add" => {
            let pattern = str_arg(a, "pattern")?;
            let field = chrona_core::rules::Field::from_key(
                &str_arg(a, "field").unwrap_or_else(|_| "app".into()),
            )
            .ok_or_else(|| anyhow::anyhow!("field must be app|title"))?;
            let category = chrona_core::model::Category::from_key(&str_arg(a, "category")?)
                .ok_or_else(|| anyhow::anyhow!("unknown category"))?;
            let id = sh.store.add_rule(&chrona_core::rules::Rule {
                id: None,
                pattern,
                field,
                category,
                priority: a
                    .get("priority")
                    .and_then(Value::as_i64)
                    .unwrap_or(100)
                    // Sanity-clamped: keep user rules inside a usable range.
                    .clamp(0, 100_000) as i32,
            })?;
            sh.rebuild_ruleset();
            Ok(json!({"id": id}))
        }
        "rule.del" => {
            let id = a
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("id required"))?;
            sh.store.remove_rule(id)?;
            sh.rebuild_ruleset();
            Ok(json!({"removed": id}))
        }

        // ----- goals -----
        "goals" => {
            let goals = sh.store.goals()?;
            let today = today_usage_map(sh);
            Ok(json!(goals
                .iter()
                .map(|g| goal_json(g, today.get(&goal_map_key(g))))
                .collect::<Vec<_>>()))
        }
        "goal.set" => {
            let kind = str_arg(a, "kind")?;
            if kind != "app" && kind != "category" && kind != "total" {
                anyhow::bail!("kind must be app|category|total");
            }
            let key = if kind == "total" {
                "total".to_string()
            } else {
                str_arg(a, "key")?
            };
            let limit = a
                .get("limit_seconds")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("limit_seconds required"))?
                // Sanity-clamped: 1 minute .. 1 day (goals are per-day).
                .clamp(60, 86_400);
            let enabled = a.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            let id = sh.store.set_goal(&kind, &key, limit, enabled)?;
            Ok(json!({"id": id}))
        }
        "goal.toggle" => {
            // Atomic single-row toggle — no read-modify-write race between
            // the UI and other API clients.
            let id = a
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("id required"))?;
            let enabled = sh
                .store
                .toggle_goal(id)?
                .ok_or_else(|| anyhow::anyhow!("no goal with id {id}"))?;
            Ok(json!({"id": id, "enabled": enabled}))
        }
        "goal.del" => {
            let id = a
                .get("id")
                .and_then(Value::as_i64)
                .ok_or_else(|| anyhow::anyhow!("id required"))?;
            sh.store.remove_goal(id)?;
            Ok(json!({"removed": id}))
        }

        // ----- settings / data -----
        "settings.get" => {
            let key = str_arg(a, "key")?;
            Ok(json!({"key": key, "value": sh.store.setting(&key)?}))
        }
        "settings.set" => {
            let key = str_arg(a, "key")?;
            let value = str_arg(a, "value")?;
            sh.store.set_setting(&key, &value)?;
            Ok(json!({"key": key, "value": value}))
        }
        "export" => {
            let to = Local::now().timestamp();
            let from = a
                .get("from")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .map(|d| day_bounds(d).0)
                .unwrap_or(0);
            let to = a
                .get("to")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .map(|d| day_bounds(d).1)
                .unwrap_or(to);
            Ok(sh.store.export_json(from, to)?)
        }
        "purge" => {
            let before = a
                .get("before")
                .and_then(Value::as_str)
                .and_then(parse_date)
                .map(|d| day_bounds(d).0)
                .ok_or_else(|| anyhow::anyhow!("before (YYYY-MM-DD) required"))?;
            let (e, af) = sh.store.purge_before(before)?;
            Ok(json!({"events_removed": e, "afk_removed": af}))
        }

        _ => anyhow::bail!("unknown command: {cmd}"),
    }
}

fn str_arg(a: &Value, key: &str) -> anyhow::Result<String> {
    a.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("{key} required"))
}

fn month_shift(y: i32, m: i32) -> (i32, i32) {
    let mut yy = y;
    let mut mm = m;
    while mm > 12 {
        mm -= 12;
        yy += 1;
    }
    while mm < 1 {
        mm += 12;
        yy -= 1;
    }
    (yy, mm)
}

fn range_totals(sh: &Shared, from: i64, to: i64) -> i64 {
    let events = sh.store.events_range(from, to).unwrap_or_default();
    let afk = sh.store.afk_range(from, to).unwrap_or_default();
    stats::total(&stats::subtract_afk(&events, &afk))
}

fn app_json(a: &chrona_core::model::AppUsage) -> Value {
    json!({"app_id": a.app_id, "seconds": a.seconds, "sessions": a.sessions})
}

fn cat_json(c: &chrona_core::model::CategoryUsage) -> Value {
    json!({"key": c.category.key(), "label": c.category.display(), "seconds": c.seconds})
}

/// Everything the UI needs for one time range.
fn range_payload(sh: &Shared, from: i64, to: i64, _top: usize) -> Value {
    let events = sh.store.events_range(from, to).unwrap_or_default();
    let afk = sh.store.afk_range(from, to).unwrap_or_default();
    let active = stats::subtract_afk(&events, &afk);
    let total = stats::total(&active);
    let afk_seconds: i64 = afk
        .iter()
        .map(|a| a.end.min(to) - a.start.max(from))
        .filter(|s| *s > 0)
        .sum();
    let apps = stats::by_app(&active);
    let rules = sh.ruleset();
    let cats = stats::by_category(&active, &rules);
    json!({
        "from": from,
        "to": to,
        "total_seconds": total,
        "afk_seconds": afk_seconds,
        "apps": apps.iter().take(50).map(app_json).collect::<Vec<_>>(),
        "app_count": apps.len(),
        "categories": cats.iter().map(cat_json).collect::<Vec<_>>(),
        "hourly": stats::hourly(&active).to_vec(),
        "longest_session": stats::longest_session(&active),
        "unlocks": stats::unlocks(&afk, from, to),
    })
}

/// Active (AFK-subtracted) segments across one day, with adjacent segments
/// of the same app merged — feeds the dashboard timeline strip that Google
/// Digital Wellbeing shows under its daily dashboard.
fn timeline_payload(sh: &Shared, from: i64, to: i64) -> Value {
    let events = sh.store.events_range(from, to).unwrap_or_default();
    let afk = sh.store.afk_range(from, to).unwrap_or_default();
    let active = stats::subtract_afk(&events, &afk);
    let mut segs: Vec<(String, i64, i64)> = Vec::new(); // (app, start, end)
    for e in active {
        let (s, t) = (e.start.max(from), e.end.min(to));
        if t <= s {
            continue;
        }
        if let Some(last) = segs.last_mut() {
            if last.0 == e.app_id && s - last.2 <= 1 {
                last.2 = t;
                continue;
            }
        }
        segs.push((e.app_id, s, t));
    }
    json!(segs
        .into_iter()
        .take(400)
        .map(|(app_id, start, end)| json!({"start": start, "end": end, "app_id": app_id}))
        .collect::<Vec<_>>())
}

fn days_payload(sh: &Shared, from: i64, to: i64) -> Value {
    let events = sh.store.events_range(from, to).unwrap_or_default();
    let afk = sh.store.afk_range(from, to).unwrap_or_default();
    let active = stats::subtract_afk(&events, &afk);
    let rules = sh.ruleset();
    json!(stats::daily_usage(&active, from, to, &rules)
        .iter()
        .map(|d| json!({
            "date": d.date,
            "seconds": d.seconds,
            "categories": d.by_category.iter().map(cat_json).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>())
}

pub(crate) fn today_usage_map(sh: &Shared) -> std::collections::HashMap<String, i64> {
    let (from, to) = day_bounds(Local::now().date_naive());
    let mut map = std::collections::HashMap::new();
    let events = sh.store.events_range(from, to).unwrap_or_default();
    let afk = sh.store.afk_range(from, to).unwrap_or_default();
    let active = stats::subtract_afk(&events, &afk);
    map.insert("total:total".into(), stats::total(&active));
    for app in stats::by_app(&active) {
        map.insert(format!("app:{}", app.app_id), app.seconds);
    }
    for cat in stats::by_category(&active, &sh.ruleset()) {
        map.insert(format!("category:{}", cat.category.key()), cat.seconds);
    }
    map
}

fn goal_map_key(g: &Goal) -> String {
    if g.kind == "total" {
        // "total:total" — matches the key produced by today_usage_map.
        format!(
            "total:{}",
            if g.key.is_empty() {
                "total"
            } else {
                g.key.as_str()
            }
        )
    } else {
        format!("{}:{}", g.kind, g.key)
    }
}

fn goal_json(g: &Goal, used: Option<&i64>) -> Value {
    json!({
        "id": g.id,
        "kind": g.kind,
        "key": g.key,
        "limit_seconds": g.limit_seconds,
        "enabled": g.enabled,
        "used_seconds": used.copied().unwrap_or(0),
    })
}
