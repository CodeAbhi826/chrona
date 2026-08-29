//! SQLite-backed storage for Chrona. Everything lives in a single database
//! file (default `$XDG_DATA_HOME/chrona/chrona.db`). Raw window events and
//! AFK sessions are stored; all aggregation happens at query time in
//! `chrona-core`, which means editing categorisation rules later re-writes
//! history for free.

use anyhow::Context;
use chrona_core::model::{AfkSession, WindowEvent};
use chrona_core::rules::{default_rules, Field, Rule};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A daily-limit goal on either a category (`kind == "category"`) or a single
/// application (`kind == "app"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: i64,
    pub kind: String,
    pub key: String,
    /// Maximum allowed seconds per day.
    pub limit_seconds: i64,
    pub enabled: bool,
}

pub struct Store {
    conn: Mutex<Connection>,
    path: PathBuf,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating data dir {}", dir.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        let store = Self {
            conn: Mutex::new(conn),
            path: path.to_path_buf(),
        };
        store.migrate()?;
        store.seed_defaults()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                start INTEGER NOT NULL,
                end INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                title TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_events_start ON events(start);
            CREATE INDEX IF NOT EXISTS idx_events_app ON events(app_id);
            CREATE TABLE IF NOT EXISTS afk (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                start INTEGER NOT NULL,
                end INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_afk_start ON afk(start);
            CREATE TABLE IF NOT EXISTS rules (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                pattern TEXT NOT NULL,
                field TEXT NOT NULL,
                category TEXT NOT NULL,
                priority INTEGER NOT NULL DEFAULT 100
            );
            CREATE TABLE IF NOT EXISTS goals (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                kind TEXT NOT NULL,
                key TEXT NOT NULL,
                limit_seconds INTEGER NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                UNIQUE(kind, key)
            );
            CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;
        Ok(())
    }

    fn seed_defaults(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM rules", [], |r| r.get(0))?;
        if count == 0 {
            for r in default_rules() {
                conn.execute(
                    "INSERT INTO rules (pattern, field, category, priority) VALUES (?1, ?2, ?3, ?4)",
                    params![r.pattern, r.field.key(), r.category.key(), r.priority],
                )?;
            }
        }
        Ok(())
    }

    // ----- events ---------------------------------------------------------

    pub fn insert_event(&self, e: &WindowEvent) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO events (start, end, app_id, title) VALUES (?1, ?2, ?3, ?4)",
            params![e.start, e.end, e.app_id, e.title],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_event_end(&self, id: i64, end: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE events SET end = ?1 WHERE id = ?2", params![end, id])?;
        Ok(())
    }

    /// Events overlapping `[from, to)` — `to` is exclusive.
    pub fn events_range(&self, from: i64, to: i64) -> anyhow::Result<Vec<WindowEvent>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT start, end, app_id, title FROM events
             WHERE start < ?2 AND end > ?1 ORDER BY start",
        )?;
        let rows = stmt
            .query_map(params![from, to], |r| {
                Ok(WindowEvent {
                    start: r.get(0)?,
                    end: r.get(1)?,
                    app_id: r.get(2)?,
                    title: r.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ----- afk --------------------------------------------------------------

    pub fn insert_afk(&self, a: &AfkSession) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO afk (start, end) VALUES (?1, ?2)",
            params![a.start, a.end],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_afk_end(&self, id: i64, end: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("UPDATE afk SET end = ?1 WHERE id = ?2", params![end, id])?;
        Ok(())
    }

    pub fn afk_range(&self, from: i64, to: i64) -> anyhow::Result<Vec<AfkSession>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT start, end FROM afk WHERE start < ?2 AND end > ?1 ORDER BY start")?;
        let rows = stmt
            .query_map(params![from, to], |r| {
                Ok(AfkSession {
                    start: r.get(0)?,
                    end: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ----- rules ------------------------------------------------------------

    pub fn rules(&self) -> anyhow::Result<Vec<Rule>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, pattern, field, category, priority FROM rules ORDER BY priority DESC, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Rule {
                    id: Some(r.get(0)?),
                    pattern: r.get(1)?,
                    field: Field::from_key(&r.get::<_, String>(2)?).unwrap_or(Field::App),
                    category: chrona_core::model::Category::from_key(&r.get::<_, String>(3)?)
                        .unwrap_or(chrona_core::model::Category::Uncategorised),
                    priority: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn add_rule(&self, r: &Rule) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO rules (pattern, field, category, priority) VALUES (?1, ?2, ?3, ?4)",
            params![r.pattern, r.field.key(), r.category.key(), r.priority],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn remove_rule(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM rules WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ----- goals --------------------------------------------------------------

    pub fn goals(&self) -> anyhow::Result<Vec<Goal>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT id, kind, key, limit_seconds, enabled FROM goals ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Goal {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    key: r.get(2)?,
                    limit_seconds: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Insert-or-update a goal keyed by (kind, key).
    pub fn set_goal(
        &self,
        kind: &str,
        key: &str,
        limit_seconds: i64,
        enabled: bool,
    ) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO goals (kind, key, limit_seconds, enabled) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(kind, key) DO UPDATE SET limit_seconds = ?3, enabled = ?4",
            params![kind, key, limit_seconds, enabled as i64],
        )?;
        let id: i64 = conn
            .query_row(
                "SELECT id FROM goals WHERE kind = ?1 AND key = ?2",
                params![kind, key],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        Ok(id)
    }

    pub fn remove_goal(&self, id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM goals WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ----- settings -------------------------------------------------------------

    pub fn setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let v = conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(v)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![key, value],
        )?;
        Ok(())
    }

    // ----- data management ---------------------------------------------------

    /// Full JSON export of everything in a range (privacy: everything is
    /// generated locally; no network is ever involved).
    pub fn export_json(&self, from: i64, to: i64) -> anyhow::Result<serde_json::Value> {
        let events = self.events_range(from, to)?;
        let afk = self.afk_range(from, to)?;
        Ok(serde_json::json!({
            "format": "chrona-export/1",
            "exported_at": chrono::Local::now().to_rfc3339(),
            "from": from,
            "to": to,
            "events": events,
            "afk": afk,
            "rules": self.rules()?,
            "goals": self.goals()?,
        }))
    }

    /// Delete everything that ended before `ts`. Returns rows removed.
    pub fn purge_before(&self, ts: i64) -> anyhow::Result<(usize, usize)> {
        let conn = self.conn.lock().unwrap();
        let e = conn.execute("DELETE FROM events WHERE end < ?1", params![ts])?;
        let a = conn.execute("DELETE FROM afk WHERE end < ?1", params![ts])?;
        Ok((e, a))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(tag: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("chrona-store-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(&dir.join("t.db")).unwrap()
    }

    #[test]
    fn events_roundtrip_and_range() {
        let s = tmp_store("events");
        s.insert_event(&WindowEvent::new(100, 200, "firefox", "a"))
            .unwrap();
        s.insert_event(&WindowEvent::new(300, 400, "code", "b"))
            .unwrap();
        let ev = s.events_range(150, 350).unwrap();
        assert_eq!(ev.len(), 2); // both overlap [150, 350)
        let ev = s.events_range(250, 260).unwrap();
        assert!(ev.is_empty());
    }

    #[test]
    fn event_end_updates() {
        let s = tmp_store("upd");
        let id = s.insert_event(&WindowEvent::new(0, 10, "x", "t")).unwrap();
        s.update_event_end(id, 99).unwrap();
        assert_eq!(s.events_range(0, 1000).unwrap()[0].end, 99);
    }

    #[test]
    fn afk_roundtrip() {
        let s = tmp_store("afk");
        let id = s.insert_afk(&AfkSession { start: 0, end: 5 }).unwrap();
        s.update_afk_end(id, 50).unwrap();
        assert_eq!(s.afk_range(0, 100).unwrap()[0].end, 50);
    }

    #[test]
    fn default_rules_are_seeded() {
        let s = tmp_store("seed");
        let rules = s.rules().unwrap();
        assert!(!rules.is_empty());
        assert!(rules.iter().all(|r| r.id.is_some()));
    }

    #[test]
    fn goals_upsert_by_kind_and_key() {
        let s = tmp_store("goals");
        let id1 = s.set_goal("app", "firefox", 3600, true).unwrap();
        let id2 = s.set_goal("app", "firefox", 1800, false).unwrap();
        assert_eq!(id1, id2);
        let goals = s.goals().unwrap();
        assert_eq!(goals.len(), 1);
        assert_eq!(goals[0].limit_seconds, 1800);
        assert!(!goals[0].enabled);
        s.remove_goal(id1).unwrap();
        assert!(s.goals().unwrap().is_empty());
    }

    #[test]
    fn settings_roundtrip() {
        let s = tmp_store("settings");
        assert!(s.setting("theme").unwrap().is_none());
        s.set_setting("theme", "dark").unwrap();
        assert_eq!(s.setting("theme").unwrap().as_deref(), Some("dark"));
        s.set_setting("theme", "material").unwrap();
        assert_eq!(s.setting("theme").unwrap().as_deref(), Some("material"));
    }
}
