pub mod archive_safety;
pub mod config;
pub mod desktop;
pub mod download;
pub mod elf;
pub mod error;
pub mod ipc;
pub mod power;
pub mod privilege;
pub mod process;
pub mod registry;
pub mod setup;
pub mod snapshot;
pub mod sources;
pub mod types;
pub mod verify;

pub use error::{Error, Result};

use std::time::Duration;

/// A thin facade most callers (CLI/daemon/GUI) use instead of reaching into
/// individual modules: it owns the config and registry, and knows how to
/// fan a query out across every enabled, available source.
pub struct Engine {
    pub config: config::Config,
    pub registry: registry::Registry,
}

/// One check in `Engine::doctor`'s report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl Engine {
    pub fn open() -> Result<Self> {
        config::ensure_dirs()?;
        Ok(Self {
            config: config::Config::load()?,
            registry: registry::Registry::open_default()?,
        })
    }

    /// Sources that are both enabled in config and actually available on
    /// this machine (binaries present).
    pub fn active_sources(&self) -> Vec<Box<dyn sources::PackageSource>> {
        sources::all_sources()
            .into_iter()
            .filter(|s| self.config.is_source_enabled(s.id()) && s.is_available())
            .collect()
    }

    pub fn list_installed_all(&self) -> Vec<(types::SourceId, Result<Vec<types::Package>>)> {
        self.active_sources()
            .iter()
            .map(|s| (s.id(), s.list_installed()))
            .collect()
    }

    /// Search every active source, ranking each source's own results so
    /// exact/prefix matches on name or id float above substring matches
    /// (stable within each tier, so adapters that already return their best
    /// guess first keep that relative order), and marking which results are
    /// already installed -- no adapter's own `search()` sets this itself
    /// (it's cheaper for them not to query installed state on every search),
    /// so it's cross-referenced here once per source instead.
    pub fn search_all(&self, query: &str) -> Vec<(types::SourceId, Result<Vec<types::Package>>)> {
        let query_lower = query.trim().to_ascii_lowercase();
        self.active_sources()
            .iter()
            .map(|s| (s.id(), search_one_source(s.as_ref(), query, &query_lower)))
            .collect()
    }

    /// Search exactly one source, e.g. for `crate search <query> --source
    /// <id>`. Shares the same installed-marking/ranking as `search_all`,
    /// rather than the CLI calling the adapter's own `search()` directly
    /// and getting a less complete result just because a source was named.
    pub fn search_one(&self, source: types::SourceId, query: &str) -> Result<Vec<types::Package>> {
        let query_lower = query.trim().to_ascii_lowercase();
        search_one_source(sources::source_by_id(source).as_ref(), query, &query_lower)
    }

    /// Check every active source for updates and refresh the on-disk cache
    /// (used by `list cached updates` so the CLI/GUI/daemon agree on the
    /// same numbers without re-querying every source on every read).
    ///
    /// This never escalates privileges: apt's `list_updates` only reads the
    /// local cache, and Flatpak/Snap's update queries are unprivileged by
    /// design. Safe to call on a timer. Call `refresh_metadata` first if you
    /// want apt's view of "what's upgradable" to reflect the latest
    /// repository state rather than whatever was last fetched.
    pub fn check_updates_all(&self) -> Result<Vec<types::UpdateInfo>> {
        let mut all = Vec::new();
        for source in self.active_sources() {
            let updates = source.list_updates().unwrap_or_default();
            self.registry.replace_update_cache(source.id(), &updates)?;
            all.extend(updates);
        }
        self.registry.set_last_checked_at(chrono::Utc::now())?;
        Ok(all)
    }

    /// Refresh repository metadata (currently: `apt-get update`). Requires
    /// root. Callers that run on an unattended timer should guard this with
    /// `privilege::passwordless_available("apt-get")` first so a background
    /// check never pops a surprise auth prompt; `crate setup sudoers`
    /// installs the narrow rule that makes it return true.
    pub fn refresh_metadata(&self) -> Result<Vec<types::OperationOutcome>> {
        let mut outcomes = Vec::new();
        if self.config.is_source_enabled(types::SourceId::Apt) && sources::apt::is_available() {
            outcomes.push(sources::apt::refresh()?);
        }
        Ok(outcomes)
    }

    /// Full details for one package: enriched metadata (size, maintainer,
    /// homepage, license where available) beyond what a bulk listing
    /// carries. One extra process spawn, meant for on-demand use (a detail
    /// view, `crate show`), not for enriching every search/list result.
    pub fn package_details(&self, source: types::SourceId, id: &str) -> Result<types::Package> {
        match source {
            types::SourceId::Apt | types::SourceId::DebFile => sources::apt::details(id),
            types::SourceId::Flatpak => sources::flatpak::details(id),
            types::SourceId::Snap => sources::snap::details(id),
            types::SourceId::Archive | types::SourceId::AppImage => self
                .active_sources()
                .into_iter()
                .find(|s| s.id() == source)
                .and_then(|s| s.list_installed().ok())
                .and_then(|pkgs| pkgs.into_iter().find(|p| p.id == id))
                .ok_or_else(|| Error::NotFound(id.to_string())),
        }
    }

    /// Why `id` is installed and what currently depends on it. Only apt has
    /// this concept today; other sources return an empty/unknown result.
    pub fn explain(&self, source: types::SourceId, id: &str) -> Result<types::ExplainInfo> {
        match source {
            types::SourceId::Apt | types::SourceId::DebFile => sources::apt::explain(id),
            _ => Ok(types::ExplainInfo::default()),
        }
    }

    /// apt packages that were pulled in as dependencies and are no longer
    /// required by anything still installed (`apt-get autoremove`'s own
    /// candidate list) -- a read-only simulation, safe to call anytime.
    pub fn list_orphans(&self) -> Result<Vec<types::Package>> {
        if self.config.is_source_enabled(types::SourceId::Apt) && sources::apt::is_available() {
            sources::apt::list_autoremovable()
        } else {
            Ok(Vec::new())
        }
    }

    pub fn autoremove(&self) -> Result<types::OperationOutcome> {
        sources::apt::autoremove()
    }

    /// Disk space used by each active source's installed packages. Packages
    /// whose size isn't cheaply knowable don't contribute to the total but
    /// are still counted, so the summary can say "and N more of unknown
    /// size" instead of silently under-reporting.
    pub fn disk_usage_summary(&self) -> Vec<types::DiskUsageEntry> {
        self.list_installed_all()
            .into_iter()
            .map(|(source, result)| {
                let packages = result.unwrap_or_default();
                let mut total_kb = 0u64;
                let mut unknown_size_count = 0usize;
                for pkg in &packages {
                    match pkg.size_kb {
                        Some(kb) => total_kb += kb,
                        None => unknown_size_count += 1,
                    }
                }
                types::DiskUsageEntry {
                    source,
                    total_kb,
                    package_count: packages.len(),
                    unknown_size_count,
                }
            })
            .collect()
    }

    /// Restore the most recent snapshot of a manually-tracked install
    /// (archive or AppImage) taken right before it was last overwritten.
    pub fn rollback(&self, source: types::SourceId, id: &str) -> Result<types::OperationOutcome> {
        let live_path = match source {
            types::SourceId::Archive => config::Paths::archive_dir().join(id),
            types::SourceId::AppImage => {
                config::Paths::appimage_dir().join(format!("{id}.AppImage"))
            }
            _ => {
                return Err(Error::UnsupportedInstallTarget(
                    "rollback is only supported for archive and AppImage installs".to_string(),
                ));
            }
        };
        snapshot::rollback(source, id, &live_path)
    }

    pub fn list_snapshots(
        &self,
        source: types::SourceId,
        id: &str,
    ) -> Result<Vec<registry::SnapshotInfo>> {
        snapshot::list_available(source, id)
    }

    /// A quick self-check of the pieces StackCrate depends on: which
    /// adapters are usable, whether the background daemon is reachable,
    /// whether unattended privilege escalation is configured, and whether
    /// the local registry database opens cleanly.
    pub fn doctor(&self) -> Vec<DoctorCheck> {
        let mut checks = Vec::new();

        for source in sources::all_sources() {
            let id = source.id();
            let enabled = self.config.is_source_enabled(id);
            let available = source.is_available();
            checks.push(DoctorCheck {
                name: format!("source: {id}"),
                ok: !enabled || available,
                detail: if !enabled {
                    "disabled in config".to_string()
                } else if available {
                    "available".to_string()
                } else {
                    "binary not found on PATH".to_string()
                },
            });
        }

        let daemon_reachable = matches!(
            ipc::request_blocking(&ipc::Request::Ping, Duration::from_secs(2)),
            Ok(ipc::Response::Pong)
        );
        checks.push(DoctorCheck {
            name: "background daemon".to_string(),
            ok: true, // optional by design; not running is a valid, supported setup
            detail: if daemon_reachable {
                "running and reachable".to_string()
            } else {
                "not running (optional: `crate setup daemon` for background update checks \
                 and notifications; everything else works fine without it)"
                    .to_string()
            },
        });

        let passwordless = privilege::passwordless_available("apt-get");
        checks.push(DoctorCheck {
            name: "unattended apt refresh/auto-update".to_string(),
            ok: true, // optional by design; never a failing check on its own
            detail: if passwordless {
                "configured".to_string()
            } else {
                "not configured (optional: `crate setup sudoers`); scheduled checks still work, \
                 just without a privileged metadata refresh or auto-install"
                    .to_string()
            },
        });

        checks.push(DoctorCheck {
            name: "registry database".to_string(),
            ok: true, // Engine::open() already succeeded if we got this far
            detail: config::Paths::registry_db().display().to_string(),
        });

        checks
    }
}

fn search_one_source(
    source: &dyn sources::PackageSource,
    query: &str,
    query_lower: &str,
) -> Result<Vec<types::Package>> {
    let mut pkgs = source.search(query)?;
    if let Ok(installed) = source.list_installed() {
        let installed_versions: std::collections::HashMap<String, String> =
            installed.into_iter().map(|p| (p.id, p.version)).collect();
        for pkg in &mut pkgs {
            if let Some(version) = installed_versions.get(&pkg.id) {
                pkg.installed = true;
                if !version.is_empty() {
                    pkg.version = version.clone();
                }
            }
        }
    }
    rank_and_dedup(&mut pkgs, query_lower);
    Ok(pkgs)
}

fn rank_and_dedup(pkgs: &mut Vec<types::Package>, query_lower: &str) {
    pkgs.sort_by_key(|p| search_rank(p, query_lower));
    let mut seen = std::collections::HashSet::new();
    pkgs.retain(|p| seen.insert(p.id.clone()));
}

fn search_rank(pkg: &types::Package, query_lower: &str) -> u8 {
    if query_lower.is_empty() {
        return 3;
    }
    let name_lower = pkg.name.to_ascii_lowercase();
    let id_lower = pkg.id.to_ascii_lowercase();
    if id_lower == *query_lower || name_lower == *query_lower {
        0
    } else if name_lower.starts_with(query_lower) || id_lower.starts_with(query_lower) {
        1
    } else if name_lower.contains(query_lower) || id_lower.contains(query_lower) {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::Package;

    fn pkg(id: &str, name: &str) -> Package {
        Package {
            id: id.to_string(),
            name: name.to_string(),
            ..Package::default()
        }
    }

    #[test]
    fn exact_match_ranks_first() {
        let mut pkgs = vec![pkg("vim-gtk3", "vim-gtk3"), pkg("vim", "vim")];
        rank_and_dedup(&mut pkgs, "vim");
        assert_eq!(pkgs[0].id, "vim");
    }

    #[test]
    fn prefix_beats_substring() {
        let mut pkgs = vec![pkg("neovim", "neovim"), pkg("vim-tiny", "vim-tiny")];
        rank_and_dedup(&mut pkgs, "vim");
        assert_eq!(pkgs[0].id, "vim-tiny");
    }

    #[test]
    fn dedups_by_id_keeping_first() {
        let mut pkgs = vec![pkg("vim", "vim one"), pkg("vim", "vim two")];
        rank_and_dedup(&mut pkgs, "vim");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "vim one");
    }
}
