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

/// Like `run_blocking`, but `work` also receives a `LineSink` it can feed
/// output lines into as they happen; those arrive on the returned receiver
/// in real time (not just once the whole operation finishes), for a live
/// progress view. The sink is dropped (closing the receiver) once `work`
/// returns, which is how the GUI's drain loop knows to stop.
pub fn run_streaming<T, F>(
    work: F,
) -> (
    impl Future<Output = Option<T>>,
    async_channel::Receiver<stackcrate_core::types::ProgressLine>,
)
where
    T: Send + 'static,
    F: FnOnce(stackcrate_core::process::LineSink) -> T + Send + 'static,
{
    let (line_tx, line_rx) = async_channel::unbounded();
    let (result_tx, result_rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let sink: stackcrate_core::process::LineSink = std::sync::Arc::new(move |stream, line| {
            let _ = line_tx.send_blocking(stackcrate_core::types::ProgressLine {
                line: line.to_string(),
                stream,
            });
        });
        let result = work(sink);
        let _ = result_tx.send_blocking(result);
    });
    (async move { result_rx.recv().await.ok() }, line_rx)
}

pub mod actions {
    use stackcrate_core::Engine;
    use stackcrate_core::config::Config;
    use stackcrate_core::process::LineSink;
    use stackcrate_core::registry::{HistoryEntry, PinInfo, SnapshotInfo};
    use stackcrate_core::sources::{
        self, appimage::AppImageInstallOptions, archive::ArchiveInstallOptions,
    };
    use stackcrate_core::types::{
        DiskUsageEntry, ExplainInfo, InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo,
    };
    use stackcrate_core::{DoctorCheck, registry::Registry};

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

    pub fn package_details(source: SourceId, id: String) -> Option<Package> {
        Engine::open().ok()?.package_details(source, &id).ok()
    }

    pub fn explain(source: SourceId, id: String) -> Option<ExplainInfo> {
        Engine::open().ok()?.explain(source, &id).ok()
    }

    pub fn changelog(name: String) -> Result<String, String> {
        sources::apt::changelog(&name).map_err(|e| e.to_string())
    }

    pub fn removal_preview(source: SourceId, id: String) -> Vec<String> {
        if source != SourceId::Apt {
            return Vec::new();
        }
        sources::apt::removal_preview(&id).unwrap_or_default()
    }

    pub fn list_orphans() -> Vec<Package> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.list_orphans().unwrap_or_default()
    }

    pub fn autoremove_streaming(sink: LineSink) -> OperationOutcome {
        run_and_log(SourceId::Apt, "autoremove", "(orphans)", || {
            sources::apt::autoremove_with(Some(sink))
        })
    }

    pub fn list_snapshots(source: SourceId, id: String) -> Vec<SnapshotInfo> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.list_snapshots(source, &id).unwrap_or_default()
    }

    pub fn rollback(source: SourceId, id: String) -> OperationOutcome {
        let Ok(engine) = Engine::open() else {
            return OperationOutcome::failed("Could not open the engine", "");
        };
        run_and_log(source, "rollback", &id, || engine.rollback(source, &id))
    }

    pub fn recent_history(limit: u32) -> Vec<HistoryEntry> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.registry.recent_history(limit).unwrap_or_default()
    }

    pub fn disk_usage_summary() -> Vec<DiskUsageEntry> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.disk_usage_summary()
    }

    pub fn doctor() -> Vec<DoctorCheck> {
        let Ok(engine) = Engine::open() else {
            return Vec::new();
        };
        engine.doctor()
    }

    #[derive(Clone, Copy)]
    pub struct SourceStatus {
        pub source: SourceId,
        pub available: bool,
        pub enabled: bool,
    }

    pub fn sources_status() -> Vec<SourceStatus> {
        let config = Config::load().unwrap_or_default();
        SourceId::all()
            .iter()
            .map(|source| SourceStatus {
                source: *source,
                available: sources::source_by_id(*source).is_available(),
                enabled: config.is_source_enabled(*source),
            })
            .collect()
    }

    pub fn set_source_enabled(source: SourceId, enabled: bool) {
        let Ok(mut config) = Config::load() else {
            return;
        };
        config
            .disabled_sources
            .retain(|s| !s.eq_ignore_ascii_case(source.as_str()));
        if !enabled {
            config.disabled_sources.push(source.as_str().to_string());
        }
        let _ = config.save();
    }

    pub fn list_pins() -> Vec<PinInfo> {
        let Ok(registry) = Registry::open_default() else {
            return Vec::new();
        };
        registry.list_pins().unwrap_or_default()
    }

    pub fn pin(source: SourceId, id: String, version: Option<String>) -> OperationOutcome {
        let Ok(registry) = Registry::open_default() else {
            return OperationOutcome::failed("Could not open the registry", "");
        };
        if let Err(e) = registry.pin(source, &id, version.as_deref()) {
            return OperationOutcome::failed(format!("Could not pin {id}"), e.to_string());
        }
        if source == SourceId::Apt {
            return sources::apt::hold(&id).unwrap_or_else(|e| {
                OperationOutcome::failed(format!("apt-mark hold {id} failed"), e.to_string())
            });
        }
        OperationOutcome::ok(format!("Pinned {id}"), "")
    }

    pub fn unpin(source: SourceId, id: String) -> OperationOutcome {
        let Ok(registry) = Registry::open_default() else {
            return OperationOutcome::failed("Could not open the registry", "");
        };
        if let Err(e) = registry.unpin(source, &id) {
            return OperationOutcome::failed(format!("Could not unpin {id}"), e.to_string());
        }
        if source == SourceId::Apt {
            return sources::apt::unhold(&id).unwrap_or_else(|e| {
                OperationOutcome::failed(format!("apt-mark unhold {id} failed"), e.to_string())
            });
        }
        OperationOutcome::ok(format!("Unpinned {id}"), "")
    }

    pub fn install_named(source: SourceId, name: String) -> OperationOutcome {
        run_and_log(source, "install", &name, || {
            sources::source_by_id(source).install(&InstallTarget::Named {
                source,
                name: name.clone(),
            })
        })
    }

    pub fn install_named_streaming(
        source: SourceId,
        name: String,
        sink: LineSink,
    ) -> OperationOutcome {
        run_and_log(source, "install", &name, || match source {
            SourceId::Apt => sources::apt::install_named_with(&name, Some(sink)),
            SourceId::Flatpak => sources::flatpak::install_named_with(&name, Some(sink)),
            SourceId::Snap => sources::snap::install_named_with(&name, Some(sink)),
            other => sources::source_by_id(other).install(&InstallTarget::Named {
                source: other,
                name: name.clone(),
            }),
        })
    }

    pub fn install_file_streaming(path: std::path::PathBuf, sink: LineSink) -> OperationOutcome {
        install_file_impl(path, Some(sink))
    }

    fn install_file_impl(path: std::path::PathBuf, sink: Option<LineSink>) -> OperationOutcome {
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
            SourceId::DebFile => sources::deb_file::install_file_with(&path, sink.clone()),
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

    pub fn remove_streaming(source: SourceId, id: String, sink: LineSink) -> OperationOutcome {
        run_and_log(source, "remove", &id, || match source {
            SourceId::Apt => sources::apt::remove_named_with(&id, Some(sink)),
            SourceId::Flatpak => sources::flatpak::remove_named_with(&id, Some(sink)),
            SourceId::Snap => sources::snap::remove_named_with(&id, Some(sink)),
            other => sources::source_by_id(other).remove(&id),
        })
    }

    pub fn update_streaming(source: SourceId, id: String, sink: LineSink) -> OperationOutcome {
        let target = if id == "(all)" {
            None
        } else {
            Some(id.as_str())
        };
        run_and_log(source, "update", &id, || match source {
            SourceId::Apt => sources::apt::update_with(target, Some(sink)),
            SourceId::Flatpak => sources::flatpak::update_with(target, Some(sink)),
            SourceId::Snap => sources::snap::update_with(target, Some(sink)),
            other => sources::source_by_id(other).update(target),
        })
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
            let _ = engine.registry.log_history(&HistoryEntry {
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
