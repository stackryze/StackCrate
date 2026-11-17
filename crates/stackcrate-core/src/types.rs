use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Identifies which adapter a package belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceId {
    Apt,
    DebFile,
    Archive,
    AppImage,
    Flatpak,
    Snap,
}

impl SourceId {
    pub fn all() -> &'static [SourceId] {
        &[
            SourceId::Apt,
            SourceId::DebFile,
            SourceId::Archive,
            SourceId::AppImage,
            SourceId::Flatpak,
            SourceId::Snap,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SourceId::Apt => "apt",
            SourceId::DebFile => "deb-file",
            SourceId::Archive => "archive",
            SourceId::AppImage => "appimage",
            SourceId::Flatpak => "flatpak",
            SourceId::Snap => "snap",
        }
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SourceId {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "apt" | "apt-dpkg" | "dpkg" => Ok(SourceId::Apt),
            "deb-file" | "deb" => Ok(SourceId::DebFile),
            "archive" | "tar" | "zip" => Ok(SourceId::Archive),
            "appimage" => Ok(SourceId::AppImage),
            "flatpak" | "flat" => Ok(SourceId::Flatpak),
            "snap" => Ok(SourceId::Snap),
            other => Err(crate::Error::Other(format!("unknown source: {other}"))),
        }
    }
}

/// A package as returned by search or list-installed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Package {
    /// Canonical id used for install/remove calls (e.g. apt package name, flatpak app id).
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub source: Option<SourceId>,
    pub installed: bool,
    /// Set when an update is available; holds the version that would be installed.
    pub available_version: Option<String>,
    pub is_security_update: bool,
    pub icon: Option<String>,
    /// Installed (or would-be-installed) size in kibibytes, when the source
    /// can report it cheaply. `None` means unknown, not zero.
    pub size_kb: Option<u64>,
    pub maintainer: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    /// apt only: `Some(true)` if the user explicitly asked for this package
    /// (as opposed to it being pulled in as a dependency). `None` when the
    /// source doesn't have this concept or it wasn't looked up.
    pub manually_installed: Option<bool>,
}

/// Why a package is installed, and what would break if it were removed.
/// Populated on demand (`Engine::explain`), not as part of every listing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExplainInfo {
    pub manually_installed: Option<bool>,
    /// Other installed packages that declare a dependency on this one.
    pub required_by: Vec<String>,
}

/// Disk space used by one source's installed packages, for the disk-usage
/// summary (`Engine::disk_usage_summary`). Packages with an unknown size
/// don't contribute to `total_kb` but are counted so the summary can say so.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsageEntry {
    pub source: SourceId,
    pub total_kb: u64,
    pub package_count: usize,
    pub unknown_size_count: usize,
}

/// Parse a human-readable size string (as printed by `flatpak info`, `snap
/// info`, etc: `"26.3 MB"`, `"4096B"`, `"1.2 GiB"`, ...) into kibibytes.
/// Returns `None` for anything that doesn't look like `<number><unit>`.
pub fn parse_size_to_kb(text: &str) -> Option<u64> {
    let text = text.trim();
    let split_at = text.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let (number, unit) = text.split_at(split_at);
    let value: f64 = number.trim().parse().ok()?;
    let unit = unit.trim().to_ascii_lowercase();
    let kib = match unit.as_str() {
        "b" | "" => value / 1024.0,
        "k" | "kb" | "kib" => value,
        "m" | "mb" | "mib" => value * 1024.0,
        "g" | "gb" | "gib" => value * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => value * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some(kib.round() as u64)
}

/// Render kibibytes back into the kind of human-readable string used across
/// the CLI and GUI (`"128.4 MB"`), picking the largest unit that keeps the
/// number readable.
pub fn format_size_kb(kb: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    let mut value = kb as f64;
    let mut unit = "KB";
    for candidate in UNITS {
        unit = candidate;
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
    }
    if unit == "KB" && value.fract() == 0.0 {
        format!("{value:.0} {unit}")
    } else {
        format!("{value:.1} {unit}")
    }
}

/// Severity classification used for update notifications and auto-apply policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateKind {
    Security,
    Normal,
}

/// A pending update for one package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub source: SourceId,
    pub id: String,
    pub name: String,
    pub current_version: String,
    pub new_version: String,
    pub kind: UpdateKind,
}

/// Result of a mutating operation (install/remove/update), returned to callers
/// after the underlying command has finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationOutcome {
    pub success: bool,
    pub summary: String,
    pub log: String,
}

impl OperationOutcome {
    pub fn ok(summary: impl Into<String>, log: impl Into<String>) -> Self {
        Self {
            success: true,
            summary: summary.into(),
            log: log.into(),
        }
    }

    pub fn failed(summary: impl Into<String>, log: impl Into<String>) -> Self {
        Self {
            success: false,
            summary: summary.into(),
            log: log.into(),
        }
    }
}

/// What the caller wants installed. `install()` on a source receives one of these
/// after `resolve_install_target` has sniffed the input.
#[derive(Debug, Clone)]
pub enum InstallTarget {
    /// A named package to be resolved through a repository-backed source (apt, flatpak, snap).
    Named { source: SourceId, name: String },
    /// A local file path already on disk (e.g. a downloaded .deb or .AppImage).
    LocalFile { source: SourceId, path: PathBuf },
    /// A remote URL that must be downloaded before installing.
    Url { source: SourceId, url: String },
}

impl InstallTarget {
    pub fn source(&self) -> SourceId {
        match self {
            InstallTarget::Named { source, .. } => *source,
            InstallTarget::LocalFile { source, .. } => *source,
            InstallTarget::Url { source, .. } => *source,
        }
    }
}

/// A short, stable identifier for a set of pending updates: same packages at
/// the same versions always produce the same fingerprint, regardless of
/// ordering. Used to decide whether a notification would be telling the user
/// something they haven't already been told.
pub fn fingerprint_updates(updates: &[UpdateInfo]) -> String {
    use std::fmt::Write;
    let mut keys: Vec<String> = updates
        .iter()
        .map(|u| format!("{}:{}:{}", u.source.as_str(), u.id, u.new_version))
        .collect();
    keys.sort_unstable();
    let mut hasher = change_fingerprint::Fnv1aHasher::default();
    for key in &keys {
        let _ = writeln!(hasher, "{key}");
    }
    hasher.hex_digest()
}

/// Tiny dependency-free FNV-1a hash used only for the notification
/// fingerprint above (not a security boundary, just change detection, so a
/// cryptographic hash would be overkill here).
mod change_fingerprint {
    use std::fmt;

    #[derive(Default)]
    pub struct Fnv1aHasher {
        state: u64,
    }

    impl Fnv1aHasher {
        pub fn hex_digest(&self) -> String {
            format!("{:016x}", self.state)
        }
    }

    impl fmt::Write for Fnv1aHasher {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            const FNV_PRIME: u64 = 0x100000001b3;
            let mut state = if self.state == 0 {
                0xcbf29ce484222325
            } else {
                self.state
            };
            for byte in s.bytes() {
                state ^= u64::from(byte);
                state = state.wrapping_mul(FNV_PRIME);
            }
            self.state = state;
            Ok(())
        }
    }
}

/// Progress/log line emitted while a long-running operation executes. The daemon
/// and CLI both consume this to stream output instead of waiting for completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressLine {
    pub line: String,
    pub stream: StreamKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamKind {
    Stdout,
    Stderr,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_bytes() {
        assert_eq!(parse_size_to_kb("4096B"), Some(4));
    }

    #[test]
    fn parses_decimal_megabytes() {
        assert_eq!(parse_size_to_kb("26.3 MB"), Some(26931));
    }

    #[test]
    fn parses_gib_case_insensitively() {
        assert_eq!(parse_size_to_kb("1GiB"), Some(1024 * 1024));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_size_to_kb("not a size"), None);
    }

    #[test]
    fn formats_small_sizes_as_whole_kb() {
        assert_eq!(format_size_kb(512), "512 KB");
    }

    #[test]
    fn formats_large_sizes_with_one_decimal() {
        assert_eq!(format_size_kb(1536), "1.5 MB");
    }
}
