pub mod appimage;
pub mod apt;
pub mod archive;
pub mod deb_file;
pub mod flatpak;
pub mod snap;
pub mod sniff;

use crate::error::{Error, Result};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo};

/// Everything a package-manager adapter must implement. Search and update
/// listing are best-effort: adapters with no remote index (archive,
/// appimage, deb-file) return an empty vector rather than erroring.
pub trait PackageSource: Send + Sync {
    fn id(&self) -> SourceId;

    /// Whether this source's underlying tool(s) are present on this system.
    fn is_available(&self) -> bool;

    fn list_installed(&self) -> Result<Vec<Package>>;

    fn search(&self, query: &str) -> Result<Vec<Package>>;

    fn list_updates(&self) -> Result<Vec<UpdateInfo>>;

    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome>;

    fn remove(&self, id: &str) -> Result<OperationOutcome>;

    /// Update one package (`Some(id)`) or everything this source manages (`None`).
    fn update(&self, id: Option<&str>) -> Result<OperationOutcome>;
}

/// Construct every known adapter, regardless of availability. Callers filter
/// with `is_available()` and `Config::is_source_enabled`.
pub fn all_sources() -> Vec<Box<dyn PackageSource>> {
    vec![
        Box::new(apt::AptSource),
        Box::new(deb_file::DebFileSource),
        Box::new(archive::ArchiveSource::new()),
        Box::new(appimage::AppImageSource::new()),
        Box::new(flatpak::FlatpakSource),
        Box::new(snap::SnapSource),
    ]
}

pub fn source_by_id(id: SourceId) -> Box<dyn PackageSource> {
    match id {
        SourceId::Apt => Box::new(apt::AptSource),
        SourceId::DebFile => Box::new(deb_file::DebFileSource),
        SourceId::Archive => Box::new(archive::ArchiveSource::new()),
        SourceId::AppImage => Box::new(appimage::AppImageSource::new()),
        SourceId::Flatpak => Box::new(flatpak::FlatpakSource),
        SourceId::Snap => Box::new(snap::SnapSource),
    }
}

/// Figure out what to install from a user-supplied string: an existing local
/// file path, an http(s) URL (downloaded into the cache dir, then sniffed the
/// same way a local file would be), or a bare package name. An explicit
/// `source_hint` always wins over sniffing.
pub fn resolve_install_target(input: &str, source_hint: Option<SourceId>) -> Result<InstallTarget> {
    let path = std::path::Path::new(input);

    if let Some(hint) = source_hint {
        if path.exists() {
            return Ok(InstallTarget::LocalFile {
                source: hint,
                path: path.to_path_buf(),
            });
        }
        if input.starts_with("http://") || input.starts_with("https://") {
            return Ok(InstallTarget::Url {
                source: hint,
                url: input.to_string(),
            });
        }
        return Ok(InstallTarget::Named {
            source: hint,
            name: input.to_string(),
        });
    }

    if path.exists() && path.is_file() {
        let kind = sniff::detect(path)?.ok_or_else(|| {
            Error::UnsupportedInstallTarget(format!(
                "could not determine the file type of {input} (pass --source to override)"
            ))
        })?;
        return Ok(InstallTarget::LocalFile {
            source: kind,
            path: path.to_path_buf(),
        });
    }

    if input.starts_with("http://") || input.starts_with("https://") {
        // Best-effort guess from the URL path so progress messages can say
        // something meaningful before the download even starts; the real
        // decision happens after downloading, via `install_url`.
        let guessed =
            sniff::detect_by_extension(std::path::Path::new(input)).unwrap_or(SourceId::Archive);
        return Ok(InstallTarget::Url {
            source: guessed,
            url: input.to_string(),
        });
    }

    // Bare name with no local file and no URL: default to apt, the primary
    // repository-backed source for this build. Flatpak/Snap must be
    // requested explicitly with --source to avoid guessing wrong between
    // ecosystems that can both plausibly own a short name.
    Ok(InstallTarget::Named {
        source: SourceId::Apt,
        name: input.to_string(),
    })
}
