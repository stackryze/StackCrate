//! Best-effort filesystem watch for "something changed outside Pallet"
//! signals, so a manual `apt install` or similar in another terminal is
//! picked up faster than waiting for the next timer tick. Debounced and
//! deliberately narrow: only a handful of well-known, stable state files are
//! watched, and a miss here just means the next scheduled check catches it
//! instead, so failures are logged and otherwise ignored.

use notify_debouncer_mini::{DebounceEventResult, new_debouncer};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, warn};

const WATCH_TARGETS: &[&str] = &[
    "/var/lib/dpkg/status",      // apt/dpkg installs, removes, upgrades
    "/var/lib/snapd/state.json", // snap installs, removes, refreshes
];

/// Spawns a blocking watcher thread and returns a receiver that gets a `()`
/// each time a debounced change fires. Returns `None` if no watch target
/// exists on this system (nothing to watch, nothing to report).
pub fn spawn() -> Option<mpsc::Receiver<()>> {
    let targets: Vec<PathBuf> = WATCH_TARGETS
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    if targets.is_empty() {
        debug!("no known package-state files found to watch");
        return None;
    }

    let (tx, rx) = mpsc::channel(1);
    std::thread::spawn(move || watch_blocking(&targets, tx));
    Some(rx)
}

fn watch_blocking(targets: &[PathBuf], tx: mpsc::Sender<()>) {
    let (notify_tx, notify_rx) = std::sync::mpsc::channel::<DebounceEventResult>();
    let mut debouncer = match new_debouncer(Duration::from_secs(5), notify_tx) {
        Ok(d) => d,
        Err(e) => {
            warn!("could not start filesystem watcher: {e}");
            return;
        }
    };

    // Watch each target's parent directory (not just the file) so an
    // atomic rename-swap write (dpkg's usual status.tmp -> status pattern)
    // is still observed, then filter events below to the exact target
    // files. Watching the directory *without* filtering would also fire on
    // unrelated sibling churn (lock files, `status-old`, trigger state,
    // etc.), which in practice retriggers fast enough to self-sustain a
    // loop against our own read-only queries touching those siblings.
    for target in targets {
        let watch_dir: &Path = target.parent().unwrap_or(target.as_path());
        if let Err(e) = debouncer
            .watcher()
            .watch(watch_dir, notify::RecursiveMode::NonRecursive)
        {
            warn!("could not watch {}: {e}", watch_dir.display());
        }
    }

    for result in notify_rx {
        match result {
            Ok(events) => {
                let hit = events.iter().find(|event| targets.contains(&event.path));
                let Some(hit) = hit else { continue };
                debug!(
                    "filesystem watch observed a relevant change at {}",
                    hit.path.display()
                );
                if tx.blocking_send(()).is_err() {
                    return; // receiver dropped; daemon is shutting down
                }
            }
            Err(e) => warn!("filesystem watch error: {e}"),
        }
    }
}
