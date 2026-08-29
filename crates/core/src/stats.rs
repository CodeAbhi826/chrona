use crate::model::{AfkSession, AppUsage, CategoryUsage, DayUsage, TitleUsage, WindowEvent};
use crate::rules::RuleSet;
use chrono::{Local, NaiveDate, TimeZone};
use std::collections::HashMap;

/// Subtract all AFK sessions from the events, splitting events where an AFK
/// session overlaps them. Order of the output follows the input events.
pub fn subtract_afk(events: &[WindowEvent], afk: &[AfkSession]) -> Vec<WindowEvent> {
    let mut out = Vec::new();
    for e in events {
        // Start with one segment covering the whole event.
        let mut segs = vec![(e.start, e.end)];
        for a in afk {
            segs = subtract_interval(&segs, a.start.max(e.start), a.end.min(e.end));
        }
        for (s, t) in segs {
            if t > s {
                out.push(WindowEvent::new(s, t, e.app_id.clone(), e.title.clone()));
            }
        }
    }
    out
}

/// Remove `[s, e)` from a list of disjoint segments.
fn subtract_interval(segs: &[(i64, i64)], s: i64, e: i64) -> Vec<(i64, i64)> {
    if e <= s {
        return segs.to_vec();
    }
    let mut out = Vec::with_capacity(segs.len() + 1);
    for &(a, b) in segs {
        if e <= a || b <= s {
            // No overlap.
            out.push((a, b));
            continue;
        }
        if s > a {
            out.push((a, s));
        }
        if e < b {
            out.push((e, b));
        }
    }
    out
}

/// Total active seconds (events must already be AFK-subtracted).
pub fn total(events: &[WindowEvent]) -> i64 {
    events.iter().map(|e| e.duration()).sum()
}

/// Longest single window session in seconds.
pub fn longest_session(events: &[WindowEvent]) -> i64 {
    events.iter().map(|e| e.duration()).max().unwrap_or(0)
}

/// Usage per application, sorted by seconds descending.
pub fn by_app(events: &[WindowEvent]) -> Vec<AppUsage> {
    let mut map: HashMap<&str, (i64, i64)> = HashMap::new();
    for e in events {
        let slot = map.entry(e.app_id.as_str()).or_insert((0, 0));
        slot.0 += e.duration();
        slot.1 += 1;
    }
    let mut v: Vec<AppUsage> = map
        .into_iter()
        .map(|(app_id, (seconds, sessions))| AppUsage {
            app_id: app_id.to_string(),
            seconds,
            sessions,
        })
        .collect();
    v.sort_by(|a, b| {
        b.seconds
            .cmp(&a.seconds)
            .then_with(|| a.app_id.cmp(&b.app_id))
    });
    v
}

/// Top window titles for one application, sorted by seconds descending.
pub fn titles_for(events: &[WindowEvent], app_id: &str) -> Vec<TitleUsage> {
    let mut map: HashMap<&str, (i64, i64)> = HashMap::new();
    for e in events.iter().filter(|e| e.app_id == app_id) {
        let slot = map.entry(e.title.as_str()).or_insert((0, 0));
        slot.0 += e.duration();
        slot.1 += 1;
    }
    let mut v: Vec<TitleUsage> = map
        .into_iter()
        .map(|(title, (seconds, sessions))| TitleUsage {
            title: title.to_string(),
            seconds,
            sessions,
        })
        .collect();
    v.sort_by(|a, b| {
        b.seconds
            .cmp(&a.seconds)
            .then_with(|| a.title.cmp(&b.title))
    });
    v
}

/// Usage per category, sorted by seconds descending.
pub fn by_category(events: &[WindowEvent], rules: &RuleSet) -> Vec<CategoryUsage> {
    let mut map: HashMap<crate::model::Category, i64> = HashMap::new();
    for e in events {
        *map.entry(rules.categorize(&e.app_id, &e.title))
            .or_insert(0) += e.duration();
    }
    let mut v: Vec<CategoryUsage> = map
        .into_iter()
        .map(|(category, seconds)| CategoryUsage { category, seconds })
        .collect();
    v.sort_by_key(|u| std::cmp::Reverse(u.seconds));
    v
}

/// Activity per local hour of day (24 buckets).
pub fn hourly(events: &[WindowEvent]) -> [i64; 24] {
    use chrono::Timelike;
    let mut buckets = [0i64; 24];
    for e in events {
        let mut t = e.start;
        while t < e.end {
            let Some(dt) = Local.timestamp_opt(t, 0).single() else {
                break;
            };
            let hour = dt.hour() as usize;
            let secs_left = 3600 - (dt.minute() as i64 * 60 + dt.second() as i64);
            let step = secs_left.min(e.end - t).max(1);
            buckets[hour.min(23)] += step;
            t += step;
        }
    }
    buckets
}

/// Split AFK-subtracted events into local days and aggregate each day.
/// `from`/`to` are unix timestamps defining an inclusive-exclusive range.
pub fn daily_usage(events: &[WindowEvent], from: i64, to: i64, rules: &RuleSet) -> Vec<DayUsage> {
    let start_date = Local
        .timestamp_opt(from, 0)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
    let end_date = Local
        .timestamp_opt(to.saturating_sub(1), 0)
        .single()
        .map(|d| d.date_naive())
        .unwrap_or(start_date);

    let mut days: Vec<DayUsage> = Vec::new();
    let mut cur = start_date;
    while cur <= end_date {
        let day_start = cur
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| {
                Local
                    .from_local_datetime(&ndt)
                    .single()
                    .map(|d| d.timestamp())
            })
            .unwrap_or(from);
        let day_end = (cur + chrono::Duration::days(1))
            .and_hms_opt(0, 0, 0)
            .and_then(|ndt| {
                Local
                    .from_local_datetime(&ndt)
                    .single()
                    .map(|d| d.timestamp())
            })
            .unwrap_or(to);

        let clamped_start = day_start.max(from);
        let clamped_end = day_end.min(to);
        // Clamp each event to the day (and the requested range) so events
        // spanning midnight are split correctly between the two days.
        let day_events: Vec<WindowEvent> = events
            .iter()
            .filter(|e| e.start < clamped_end && e.end > clamped_start)
            .map(|e| {
                WindowEvent::new(
                    e.start.max(clamped_start),
                    e.end.min(clamped_end),
                    e.app_id.clone(),
                    e.title.clone(),
                )
            })
            .collect();
        let seconds = total(&day_events);
        let by_category = by_category(&day_events, rules);
        days.push(DayUsage {
            date: cur.format("%Y-%m-%d").to_string(),
            seconds,
            by_category,
        });
        cur += chrono::Duration::days(1);
    }
    days
}

/// "Unlocks": the number of times the user came back from AFK in the range
/// (mirrors the phone "unlocks" counter in Google Digital Wellbeing).
pub fn unlocks(afk: &[AfkSession], from: i64, to: i64) -> i64 {
    afk.iter().filter(|a| a.end > from && a.end <= to).count() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Category;
    use crate::rules::default_rules;
    use chrono::Timelike;

    fn ev(start: i64, end: i64, app: &str) -> WindowEvent {
        WindowEvent::new(start, end, app, "t")
    }

    #[test]
    fn afk_subtraction_splits_events() {
        let events = vec![ev(0, 3600, "firefox")];
        let afk = vec![AfkSession {
            start: 900,
            end: 1800,
        }];
        let out = subtract_afk(&events, &afk);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start, 0);
        assert_eq!(out[0].end, 900);
        assert_eq!(out[1].start, 1800);
        assert_eq!(out[1].end, 3600);
        assert_eq!(total(&out), 3600 - 900);
    }

    #[test]
    fn afk_covering_whole_event_removes_it() {
        let events = vec![ev(0, 100, "x")];
        let afk = vec![AfkSession { start: 0, end: 200 }];
        assert!(subtract_afk(&events, &afk).is_empty());
    }

    #[test]
    fn by_app_merges_and_counts_sessions() {
        let events = vec![ev(0, 60, "a"), ev(120, 180, "b"), ev(300, 400, "a")];
        let apps = by_app(&events);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].app_id, "a"); // 160s total beats b's 60s
        assert_eq!(apps[0].seconds, 160);
        assert_eq!(apps[0].sessions, 2);
        assert_eq!(apps[1].sessions, 1);
    }

    #[test]
    fn hourly_buckets_a_two_hour_event() {
        // Use a fixed UTC-ish base via Local — pick an arbitrary timestamp and
        // compute its local hour so the test is timezone-independent.
        let base = Local::now().timestamp();
        let base_hour = Local.timestamp_opt(base, 0).single().unwrap().hour() as i64;
        // Build an event exactly aligned to the start of `base_hour`.
        let aligned = base - (base % 3600);
        let events = vec![ev(aligned, aligned + 7200, "x")]; // two full hours
        let h = hourly(&events);
        let total: i64 = h.iter().sum();
        assert_eq!(total, 7200);
        assert_eq!(h[(base_hour as usize + 24) % 24], 3600);
        assert_eq!(h[((base_hour + 1) as usize + 24) % 24], 3600);
    }

    #[test]
    fn daily_usage_spans_days() {
        let rules = RuleSet::compile(&default_rules()).unwrap();
        // One event spanning midnight local time.
        let midnight = Local::now().date_naive().and_hms_opt(23, 30, 0).unwrap();
        let start = Local
            .from_local_datetime(&midnight)
            .single()
            .unwrap()
            .timestamp();
        let events = vec![WindowEvent::new(start, start + 3600, "firefox", "t")];
        let days = daily_usage(&events, start - 60, start + 3600 + 60, &rules);
        assert!(days.len() >= 2);
        let sum: i64 = days.iter().map(|d| d.seconds).sum();
        assert_eq!(sum, 3600);
        // Categorised as browser usage.
        let all_cats: Vec<&CategoryUsage> =
            days.iter().flat_map(|d| d.by_category.iter()).collect();
        assert!(all_cats.iter().any(|c| c.category == Category::Browsers));
    }

    #[test]
    fn unlocks_counts_resumes() {
        let afk = vec![
            AfkSession { start: 0, end: 100 },
            AfkSession {
                start: 200,
                end: 300,
            },
        ];
        assert_eq!(unlocks(&afk, 0, 1000), 2);
        assert_eq!(unlocks(&afk, 0, 150), 1);
    }

    #[test]
    fn titles_ranked_by_time() {
        let events = vec![
            WindowEvent::new(0, 100, "code", "main.rs"),
            WindowEvent::new(200, 500, "code", "lib.rs"),
        ];
        let t = titles_for(&events, "code");
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].title, "lib.rs");
        assert_eq!(t[0].seconds, 300);
    }
}
