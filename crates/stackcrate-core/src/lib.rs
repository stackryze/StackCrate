pub mod archive_safety;
pub mod config;
pub mod desktop;
pub mod download;
pub mod error;
pub mod ipc;
pub mod privilege;
pub mod process;
pub mod registry;
pub mod sources;
pub mod types;
pub mod verify;

pub use error::{Error, Result};

/// A thin facade most callers (CLI/daemon/GUI) use instead of reaching into
/// individual modules: it owns the config and registry, and knows how to
/// fan a query out across every enabled, available source.
pub struct Engine {
    pub config: config::Config,
    pub registry: registry::Registry,
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

    pub fn search_all(&self, query: &str) -> Vec<(types::SourceId, Result<Vec<types::Package>>)> {
        self.active_sources()
            .iter()
            .map(|s| (s.id(), s.search(query)))
            .collect()
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
}
