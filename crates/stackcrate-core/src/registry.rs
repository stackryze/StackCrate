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

/// One package held back from bulk updates, optionally pinned to a specific
/// version (apt: also enforced at the OS level via `apt-mark hold`, so a
/// plain `apt upgrade` run outside StackCrate respects it too).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinInfo {
    pub source: SourceId,
    pub package_id: String,
    pub pinned_version: Option<String>,
}

/// A previous on-disk state of a manually-tracked install (archive or
/// AppImage), saved right before it was overwritten, so `crate rollback` can
/// restore it. `manual_install` is the full registry row as it was at
/// snapshot time, so restoring never has to regenerate a desktop file, icon,
/// or exec path by guesswork.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotInfo {
    pub id: i64,
    pub snapshot_path: String,
    pub manual_install: ManualInstall,
    pub created_at: DateTime<Utc>,
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
        let conn = reg.connect()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
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

    pub fn pin(&self, source: SourceId, package_id: &str, version: Option<&str>) -> Result<()> {
        self.connect()?.execute(
            "INSERT INTO pins (source, package_id, pinned_version) VALUES (?1, ?2, ?3)
             ON CONFLICT(source, package_id) DO UPDATE SET pinned_version = excluded.pinned_version",
            params![source.as_str(), package_id, version],
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

    pub fn list_pins(&self) -> Result<Vec<PinInfo>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare("SELECT source, package_id, pinned_version FROM pins")?;
        let rows = stmt.query_map([], |row| {
            let source: String = row.get(0)?;
            Ok(PinInfo {
                source: source.parse().unwrap_or(SourceId::Apt),
                package_id: row.get(1)?,
                pinned_version: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    // ---- snapshots (archive/AppImage rollback) --------------------------

    pub fn add_snapshot(&self, snapshot_path: &str, manual_install: &ManualInstall) -> Result<i64> {
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO snapshots (source, package_id, snapshot_path, manual_install_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                manual_install.source.as_str(),
                manual_install.id,
                snapshot_path,
                serde_json::to_string(manual_install)?,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn latest_snapshot(
        &self,
        source: SourceId,
        package_id: &str,
    ) -> Result<Option<SnapshotInfo>> {
        Ok(self.list_snapshots(source, package_id)?.into_iter().next())
    }

    pub fn list_snapshots(&self, source: SourceId, package_id: &str) -> Result<Vec<SnapshotInfo>> {
        let conn = self.connect()?;
        let mut stmt = conn.prepare(
            "SELECT id, snapshot_path, manual_install_json, created_at
             FROM snapshots WHERE source = ?1 AND package_id = ?2 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![source.as_str(), package_id], row_to_snapshot)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn remove_snapshot(&self, id: i64) -> Result<()> {
        self.connect()?
            .execute("DELETE FROM snapshots WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn row_to_snapshot(row: &rusqlite::Row) -> rusqlite::Result<SnapshotInfo> {
    let manual_json: String = row.get(2)?;
    let created_at: String = row.get(3)?;
    let manual_install: ManualInstall = serde_json::from_str(&manual_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(SnapshotInfo {
        id: row.get(0)?,
        snapshot_path: row.get(1)?,
        manual_install,
        created_at: DateTime::parse_from_rfc3339(&created_at)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
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
    pinned_version TEXT,
    PRIMARY KEY (source, package_id)
);
CREATE TABLE IF NOT EXISTS snapshots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    package_id TEXT NOT NULL,
    snapshot_path TEXT NOT NULL,
    manual_install_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Schema changes for databases created before this column/table existed.
/// `ALTER TABLE ... ADD COLUMN` has no `IF NOT EXISTS` in SQLite, so a
/// "duplicate column" failure here just means a previous run already
/// applied it, and is ignored; anything else is a real error.
fn migrate(conn: &Connection) -> Result<()> {
    if let Err(e) = conn.execute("ALTER TABLE pins ADD COLUMN pinned_version TEXT", []) {
        let message = e.to_string();
        if !message.contains("duplicate column name") {
            return Err(e.into());
        }
    }
    Ok(())
}
