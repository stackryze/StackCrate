//! Shared "snapshot before overwrite" + rollback logic for the two install
//! kinds StackCrate tracks itself (archives, AppImages). There's no
//! underlying package manager to ask for a previous version for these, so
//! reinstalling over an existing one first moves the old on-disk artifact
//! aside (a cheap rename, not a copy -- see `Paths::snapshot_dir`) and
//! records it in the registry, so `crate rollback` can restore it later.
//! Every destructive change snapshots whatever it's about to replace first,
//! including a rollback itself, so rolling back is always itself undoable by
//! rolling back again.

use crate::config::Paths;
use crate::error::{Error, Result};
use crate::registry::{ManualInstall, Registry};
use crate::types::{OperationOutcome, SourceId};
use chrono::Utc;
use std::path::{Path, PathBuf};

fn snapshot_root(source: SourceId) -> PathBuf {
    Paths::snapshot_dir().join(source.as_str())
}

/// If `live_path` currently exists, move it into a dated snapshot directory
/// and record it in the registry. A no-op (not an error) when there's
/// nothing at `live_path` yet, i.e. this is a fresh install, not an
/// overwrite.
pub fn snapshot_if_present(
    registry: &Registry,
    manual: &ManualInstall,
    live_path: &Path,
) -> Result<()> {
    if !live_path.exists() {
        return Ok(());
    }
    let dir = snapshot_root(manual.source).join(&manual.id);
    std::fs::create_dir_all(&dir)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ").to_string();
    let file_name = live_path
        .file_name()
        .map(|n| n.to_owned())
        .unwrap_or_else(|| std::ffi::OsString::from("snapshot"));
    let dest = dir.join(format!("{stamp}-{}", file_name.to_string_lossy()));
    std::fs::rename(live_path, &dest)?;
    registry.add_snapshot(&dest.to_string_lossy(), manual)?;
    Ok(())
}

/// Restore the most recent snapshot for `(source, id)` back over whatever is
/// currently at `live_path`.
pub fn rollback(source: SourceId, id: &str, live_path: &Path) -> Result<OperationOutcome> {
    let registry = Registry::open_default()?;
    let Some(snapshot) = registry.latest_snapshot(source, id)? else {
        return Err(Error::NotFound(format!(
            "no snapshot available to roll back to for '{id}'"
        )));
    };

    if let Some(current) = registry.get_manual(id)? {
        snapshot_if_present(&registry, &current, live_path)?;
    } else if live_path.exists() {
        // Not currently tracked by the registry (shouldn't normally happen),
        // but still don't silently clobber whatever is on disk.
        let _ = std::fs::remove_file(live_path).or_else(|_| std::fs::remove_dir_all(live_path));
    }

    let snapshot_path = PathBuf::from(&snapshot.snapshot_path);
    if let Some(parent) = live_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&snapshot_path, live_path)?;
    registry.upsert_manual(&snapshot.manual_install)?;
    registry.remove_snapshot(snapshot.id)?;

    Ok(OperationOutcome::ok(
        format!(
            "Rolled back {} to the version from {}",
            snapshot.manual_install.name,
            snapshot
                .created_at
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M"),
        ),
        format!(
            "restored {} -> {}",
            snapshot_path.display(),
            live_path.display()
        ),
    ))
}

/// Every snapshot available for `(source, id)`, most recent first.
pub fn list_available(source: SourceId, id: &str) -> Result<Vec<crate::registry::SnapshotInfo>> {
    Registry::open_default()?.list_snapshots(source, id)
}
