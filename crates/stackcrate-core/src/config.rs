//! Configuration and well-known XDG-style paths. Everything lives under
//! `$XDG_CONFIG_HOME/stackcrate` (settings), `$XDG_DATA_HOME/stackcrate` (registry,
//! manual installs) and `$XDG_CACHE_HOME/stackcrate` (logs, download cache).

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UpdatePolicy {
    /// Only notify; nothing is installed automatically. Default.
    #[default]
    NotifyOnly,
    /// Security/important updates install automatically; everything else notifies.
    SecurityOnly,
    /// All detected updates install automatically on schedule.
    AutoAll,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// Minutes between background checks.
    #[serde(default = "default_interval")]
    pub check_interval_minutes: u64,
    #[serde(default)]
    pub update_policy: UpdatePolicy,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    /// Sources to skip even if detected as available.
    #[serde(default)]
    pub disabled_sources: Vec<String>,
    #[serde(default)]
    pub require_checksum_for_appimage: bool,
    #[serde(default)]
    pub require_checksum_for_archive: bool,
}

fn default_interval() -> u64 {
    60
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            check_interval_minutes: default_interval(),
            update_policy: UpdatePolicy::default(),
            notifications_enabled: true,
            disabled_sources: Vec::new(),
            require_checksum_for_appimage: false,
            require_checksum_for_archive: false,
        }
    }
}

pub struct Paths;

impl Paths {
    pub fn config_dir() -> PathBuf {
        dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stackcrate")
    }

    pub fn data_dir() -> PathBuf {
        dirs_next::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stackcrate")
    }

    pub fn cache_dir() -> PathBuf {
        dirs_next::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stackcrate")
    }

    pub fn runtime_dir() -> PathBuf {
        dirs_next::runtime_dir().unwrap_or_else(std::env::temp_dir)
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn registry_db() -> PathBuf {
        Self::data_dir().join("registry.sqlite3")
    }

    pub fn appimage_dir() -> PathBuf {
        home_dir().join(".local/opt/stackcrate/appimages")
    }

    pub fn archive_dir() -> PathBuf {
        home_dir().join(".local/opt/stackcrate/apps")
    }

    pub fn bin_dir() -> PathBuf {
        home_dir().join(".local/bin")
    }

    pub fn applications_dir() -> PathBuf {
        Self::data_dir_root().join("applications")
    }

    pub fn icon_dir() -> PathBuf {
        Self::data_dir_root().join("icons/hicolor/512x512/apps")
    }

    /// `$XDG_DATA_HOME` itself (not the `/stackcrate` subdirectory), needed for
    /// `applications` and `icons` which are shared desktop-wide locations.
    fn data_dir_root() -> PathBuf {
        dirs_next::data_dir().unwrap_or_else(|| home_dir().join(".local/share"))
    }

    pub fn socket_path() -> PathBuf {
        Self::runtime_dir().join("stackcrate.sock")
    }

    pub fn daemon_log() -> PathBuf {
        Self::cache_dir().join("daemon.log")
    }
}

fn home_dir() -> PathBuf {
    dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn ensure_dirs() -> Result<()> {
    for dir in [
        Paths::config_dir(),
        Paths::data_dir(),
        Paths::cache_dir(),
        Paths::appimage_dir(),
        Paths::archive_dir(),
        Paths::bin_dir(),
        Paths::applications_dir(),
        Paths::icon_dir(),
    ] {
        std::fs::create_dir_all(dir)?;
    }
    Ok(())
}

impl Config {
    pub fn load() -> Result<Self> {
        Self::load_from(&Paths::config_file())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Paths::config_file())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn is_source_enabled(&self, source: crate::types::SourceId) -> bool {
        !self
            .disabled_sources
            .iter()
            .any(|s| s.eq_ignore_ascii_case(source.as_str()))
    }
}
