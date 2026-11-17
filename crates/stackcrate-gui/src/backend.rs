//! Bridges blocking `stackcrate-core` calls (subprocess spawns, file IO) onto a
//! background thread and delivers the result back on the GTK main context,
//! so no package operation ever blocks the UI thread.

use std::future::Future;

/// Runs `work` on a plain OS thread and returns a future (awaitable from a
/// `glib::spawn_future_local` context) that resolves with its result on the
/// GTK main thread.
pub fn run_blocking<T, F>(work: F) -> impl Future<Output = Option<T>>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let result = work();
        let _ = tx.send_blocking(result);
    });
    async move { rx.recv().await.ok() }
}

pub mod actions {
    use stackcrate_core::Engine;
    use stackcrate_core::sources::{
        self, appimage::AppImageInstallOptions, archive::ArchiveInstallOptions,
    };
    use stackcrate_core::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo};

    pub fn search_all(query: String) -> Vec<Package> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine
            .search_all(&query)
            .into_iter()
            .flat_map(|(_, result)| result.unwrap_or_default())
            .collect()
    }

    pub fn list_installed_all() -> Vec<Package> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine
            .list_installed_all()
            .into_iter()
            .flat_map(|(_, result)| result.unwrap_or_default())
            .collect()
    }

    pub fn cached_updates() -> Vec<UpdateInfo> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.registry.cached_updates().unwrap_or_default()
    }

    pub fn check_updates_now() -> Vec<UpdateInfo> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.check_updates_all().unwrap_or_default()
    }

    pub fn install_named(source: SourceId, name: String) -> OperationOutcome {
        run_and_log(source, "install", &name, || {
            sources::source_by_id(source).install(&InstallTarget::Named {
                source,
                name: name.clone(),
            })
        })
    }

    pub fn install_file(path: std::path::PathBuf) -> OperationOutcome {
        let Some(kind) = sources::sniff::detect(&path).ok().flatten() else {
            return OperationOutcome::failed(
                "Could not determine file type".to_string(),
                format!("{}", path.display()),
            );
        };
        let label = path.display().to_string();
        run_and_log(kind, "install", &label, || match kind {
            SourceId::Archive => {
                sources::archive::install_archive(&path, &ArchiveInstallOptions::default())
            }
            SourceId::AppImage => {
                sources::appimage::install_appimage(&path, &AppImageInstallOptions::default())
            }
            other => sources::source_by_id(other).install(&InstallTarget::LocalFile {
                source: other,
                path: path.clone(),
            }),
        })
    }

    pub fn remove(source: SourceId, id: String) -> OperationOutcome {
        run_and_log(source, "remove", &id, || {
            sources::source_by_id(source).remove(&id)
        })
    }

    pub fn apply_updates(
        items: Vec<(SourceId, String)>,
    ) -> Vec<(SourceId, String, OperationOutcome)> {
        let mut results = Vec::new();
        if items.is_empty() {
            let Ok(engine) = Engine::open() else {
                return results;
            };
            let sources_with_updates: std::collections::BTreeSet<SourceId> = engine
                .registry
                .cached_updates()
                .unwrap_or_default()
                .into_iter()
                .map(|u| u.source)
                .collect();
            for source in sources_with_updates {
                let outcome = run_and_log(source, "update", "(all)", || {
                    sources::source_by_id(source).update(None)
                });
                results.push((source, "(all)".to_string(), outcome));
            }
        } else {
            for (source, id) in items {
                let outcome = run_and_log(source, "update", &id, || {
                    sources::source_by_id(source).update(Some(&id))
                });
                results.push((source, id, outcome));
            }
        }
        results
    }

    fn run_and_log(
        source: SourceId,
        action: &str,
        package: &str,
        f: impl FnOnce() -> stackcrate_core::Result<OperationOutcome>,
    ) -> OperationOutcome {
        let outcome = match f() {
            Ok(outcome) => outcome,
            Err(e) => OperationOutcome::failed(format!("{action} failed"), e.to_string()),
        };
        if let Ok(engine) = Engine::open() {
            let _ = engine
                .registry
                .log_history(&stackcrate_core::registry::HistoryEntry {
                    timestamp: chrono::Utc::now(),
                    source: source.as_str().to_string(),
                    action: action.to_string(),
                    package: package.to_string(),
                    success: outcome.success,
                    detail: outcome.summary.clone(),
                });
        }
        outcome
    }
}
