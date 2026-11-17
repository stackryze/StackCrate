//! SQLite-backed local state: manually-installed apps (archives/AppImages)
//! that have no system package database of their own, the update-check
//! cache the daemon populates, operation history/audit log, and pinned
//! (held-back) packages.
//!
//! Each call opens a short-lived connection in WAL mode so the CLI, daemon,
//! and GUI can all touch the same database file concurrently without a
//! shared in-process lock.

use crate::config::Paths;
use crate::error::Result;
use crate::types::{SourceId, UpdateInfo, UpdateKind};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub struct Registry {
    db_path: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualInstall {
    pub id: String,
    pub source: SourceId,
    pub name: String,
    pub version: String,
    pub path: String,
    pub exec_path: Option<String>,
    pub desktop_file: Option<String>,
    pub icon_path: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp: DateTime<Utc>,
    pub source: String,
    pub action: String,
    pub package: String,
    pub success: bool,
    pub detail: String,
}

impl Registry {
    pub fn open_default() -> Result<Self> {
        Self::open(Paths::registry_db())
    }

    pub fn open(db_path: std::path::PathBuf) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let reg = Registry { db_path };
        reg.connect()?.execute_batch(SCHEMA)?;
        Ok(reg)
    }

    fn connect(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        Ok(conn)
    }

    // ---- manual installs -------------------------------------------------

    pub fn upsert_manual(&self, entry: &ManualInstall) -> Result<()> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO manual_installs
                (id, source, name, version, path, exec_path, desktop_file, icon_path, installed_at, source_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                source=excluded.source, name=excluded.name, version=excluded.version,
                path=excluded.path, exec_path=excluded.exec_path, desktop_file=excluded.desktop_file,
                icon_path=excluded.icon_path, installed_at=excluded.installed_at, source_url=excluded.source_url",
            params![
                entry.id,
                entry.source.as_str(),
                entry.name,
                entry.version,
                entry.path,
                entry.exec_path,
                entry.desktop_file,
                entry.icon_path,
                entry.installed_at.to_rfc3339(),
                entry.source_url,
            ],
        )?;
        Ok(())
    }

    pub fn remove_manual(&self, id: &str) -> Result<Option<ManualInstall>> {
        let existing = self.get_manual(id)?;
        if existing.is_some() {
            self.connect()?
                .execute("DELETE FROM manual_installs WHERE id = ?1", params![id])?;
        }
        Ok(existing)
    }

    pub fn get_manual(&self, id: &str) -> Result<Option<ManualInstall>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, name, version, path, exec_path, desktop_file, icon_path, installed_at, source_url
             FROM manual_installs WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_manual(row)?))
        } else {
            Ok(None)
        }
    }

    pub fn list_manual(&self, source: Option<SourceId>) -> Result<Vec<ManualInstall>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, name, version, path, exec_path, desktop_file, icon_path, installed_at, source_url
             FROM manual_installs ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], row_to_manual)?;
        let mut out = Vec::new();
        for row in rows {
            let entry = row?;
            if source.map(|s| s == entry.source).unwrap_or(true) {
                out.push(entry);
            }
        }
        Ok(out)
    }

    // ---- update cache ------------------------------------------------

    pub fn replace_update_cache(&self, source: SourceId, updates: &[UpdateInfo]) -> Result<()> {
        let mut conn = self.connect()?;
        let tx = conn.transaction()?;
        tx.execute(
            "DELETE FROM update_cache WHERE source = ?1",
            params![source.as_str()],
        )?;
        let now = Utc::now().to_rfc3339();
        for u in updates {
            tx.execute(
                "INSERT INTO update_cache (source, package_id, name, current_version, new_version, kind, checked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    source.as_str(),
                    u.id,
                    u.name,
                    u.current_version,
                    u.new_version,
                    match u.kind {
                        UpdateKind::Security => "security",
                        UpdateKind::Normal => "normal",
                    },
                    now,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn cached_updates(&self) -> Result<Vec<UpdateInfo>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT source, package_id, name, current_version, new_version, kind FROM update_cache
             ORDER BY source, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            let kind: String = row.get(5)?;
            Ok(UpdateInfo {
                source: source.parse().unwrap_or(SourceId::Apt),
                id: row.get(1)?,
                name: row.get(2)?,
                current_version: row.get(3)?,
                new_version: row.get(4)?,
                kind: if kind == "security" {
                    UpdateKind::Security
                } else {
                    UpdateKind::Normal
                },
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn last_checked_at(&self) -> Result<Option<DateTime<Utc>>> {
        let conn = self.connect()?;
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_checked_at'",
                [],
                |r| r.get(0),
            )
            .ok();
        Ok(value.and_then(|v| {
            DateTime::parse_from_rfc3339(&v)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }))
    }

    pub fn set_last_checked_at(&self, when: DateTime<Utc>) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO meta (key, value) VALUES ('last_checked_at', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![when.to_rfc3339()],
        )?;
        Ok(())
    }

    /// A short fingerprint of the last update set the daemon actually sent a
    /// notification for, so unchanged pending updates don't re-notify on
    /// every single check interval.
    pub fn last_notified_fingerprint(&self) -> Result<Option<String>> {
        let conn = self.connect()?;
        Ok(conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_notified_fingerprint'",
                [],
                |r| r.get(0),
            )
            .ok())
    }

    pub fn set_last_notified_fingerprint(&self, fingerprint: &str) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO meta (key, value) VALUES ('last_notified_fingerprint', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![fingerprint],
        )?;
        Ok(())
    }

    // ---- history -----------------------------------------------------

    pub fn log_history(&self, entry: &HistoryEntry) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO history (timestamp, source, action, package, success, detail)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                entry.timestamp.to_rfc3339(),
                entry.source,
                entry.action,
                entry.package,
                entry.success as i64,
                entry.detail,
            ],
        )?;
        Ok(())
    }

    pub fn recent_history(&self, limit: u32) -> Result<Vec<HistoryEntry>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT timestamp, source, action, package, success, detail
             FROM history ORDER BY id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| {
            let ts: String = row.get(0)?;
            Ok(HistoryEntry {
                timestamp: DateTime::parse_from_rfc3339(&ts)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                source: row.get(1)?,
                action: row.get(2)?,
                package: row.get(3)?,
                success: row.get::<_, i64>(4)? != 0,
                detail: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    // ---- pins ----------------------------------------------------------

    pub fn pin(&self, source: SourceId, package_id: &str) -> Result<()> {
        self.connect()?.execute(
            "INSERT OR IGNORE INTO pins (source, package_id) VALUES (?1, ?2)",
            params![source.as_str(), package_id],
        )?;
        Ok(())
    }

    pub fn unpin(&self, source: SourceId, package_id: &str) -> Result<()> {
        self.connect()?.execute(
            "DELETE FROM pins WHERE source = ?1 AND package_id = ?2",
            params![source.as_str(), package_id],
        )?;
        Ok(())
    }

    pub fn is_pinned(&self, source: SourceId, package_id: &str) -> Result<bool> {
        let conn = self.connect()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM pins WHERE source = ?1 AND package_id = ?2",
            params![source.as_str(), package_id],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn list_pins(&self) -> Result<Vec<(SourceId, String)>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT source, package_id FROM pins")?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            Ok((source, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (source, id) = row?;
            out.push((source.parse().unwrap_or(SourceId::Apt), id));
        }
        Ok(out)
    }
}

fn row_to_manual(row: &rusqlite::Row) -> rusqlite::Result<ManualInstall> {
    let source: String = row.get(1)?;
    let installed_at: String = row.get(8)?;
    Ok(ManualInstall {
        id: row.get(0)?,
        source: source.parse().unwrap_or(SourceId::Archive),
        name: row.get(2)?,
        version: row.get(3)?,
        path: row.get(4)?,
        exec_path: row.get(5)?,
        desktop_file: row.get(6)?,
        icon_path: row.get(7)?,
        installed_at: DateTime::parse_from_rfc3339(&installed_at)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        source_url: row.get(9)?,
    })
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS manual_installs (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    name TEXT NOT NULL,
    version TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    exec_path TEXT,
    desktop_file TEXT,
    icon_path TEXT,
    installed_at TEXT NOT NULL,
    source_url TEXT
);
CREATE TABLE IF NOT EXISTS update_cache (
    source TEXT NOT NULL,
    package_id TEXT NOT NULL,
    name TEXT NOT NULL,
    current_version TEXT NOT NULL DEFAULT '',
    new_version TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL DEFAULT 'normal',
    checked_at TEXT NOT NULL,
    PRIMARY KEY (source, package_id)
);
CREATE TABLE IF NOT EXISTS history (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    source TEXT NOT NULL,
    action TEXT NOT NULL,
    package TEXT NOT NULL,
    success INTEGER NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS pins (
    source TEXT NOT NULL,
    package_id TEXT NOT NULL,
    PRIMARY KEY (source, package_id)
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";
