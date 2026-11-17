//! Manual installs for plain archives (`.tar`, `.tar.gz`, `.tar.xz`,
//! `.tar.bz2`, `.zip`). There is no package database for these, so Pallet
//! tracks them itself in the local registry: where they were extracted, what
//! (if anything) got a `.desktop` launcher, and what to clean up on removal.

use crate::archive_safety;
use crate::config::Paths;
use crate::desktop;
use crate::error::{Error, Result};
use crate::registry::{ManualInstall, Registry};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo};
use chrono::Utc;
use std::path::{Path, PathBuf};

pub struct ArchiveSource;

impl ArchiveSource {
    pub fn new() -> Self {
        ArchiveSource
    }
}

impl Default for ArchiveSource {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ArchiveInstallOptions {
    /// Display name; defaults to the archive's file stem.
    pub name: Option<String>,
    /// Path to the main executable, relative to the extracted directory.
    /// Auto-detected when not given.
    pub exec_relative: Option<String>,
    /// Path to an icon file to install alongside the launcher.
    pub icon: Option<PathBuf>,
    /// Skip creating a `.desktop` launcher / bin symlink even if a single
    /// executable is found.
    pub no_launcher: bool,
}

fn default_name_from_path(path: &Path) -> String {
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("app");
    let mut stem = file_name.to_string();
    for suffix in [
        ".tar.gz", ".tar.xz", ".tar.bz2", ".tgz", ".txz", ".tbz2", ".tar", ".zip",
    ] {
        if let Some(stripped) = stem
            .to_ascii_lowercase()
            .strip_suffix(suffix)
            .map(|s| s.len())
        {
            stem.truncate(stripped);
            break;
        }
    }
    stem
}

/// Find a plausible "main" executable inside an extracted archive: an
/// executable regular file at the top level (or one level deep, common for
/// `name-1.2.3/bin/name` style layouts) whose name resembles `want_name`,
/// falling back to the only executable found if there's just one.
fn find_main_executable(root: &Path, want_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    collect_executables(root, 0, 3, &mut candidates);
    if candidates.is_empty() {
        return None;
    }
    let want = want_name.to_ascii_lowercase();
    if let Some(exact) = candidates.iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.to_ascii_lowercase() == want)
            .unwrap_or(false)
    }) {
        return Some(exact.clone());
    }
    if candidates.len() == 1 {
        return candidates.into_iter().next();
    }
    None
}

fn collect_executables(dir: &Path, depth: u32, max_depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if depth < max_depth {
                collect_executables(&path, depth + 1, max_depth, out);
            }
            continue;
        }
        if file_type.is_file() && is_executable(&path) {
            out.push(path);
        }
    }
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

pub fn install_archive(path: &Path, opts: &ArchiveInstallOptions) -> Result<OperationOutcome> {
    if !path.exists() {
        return Err(Error::InvalidPath(format!(
            "{} does not exist",
            path.display()
        )));
    }
    let name = opts
        .name
        .clone()
        .unwrap_or_else(|| default_name_from_path(path));
    let slug = desktop::slugify(&name)?;
    let dest = Paths::archive_dir().join(&slug);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }
    archive_safety::extract_any(path, &dest)?;

    let exec_path = if opts.no_launcher {
        None
    } else if let Some(rel) = &opts.exec_relative {
        let candidate = dest.join(rel);
        if !candidate.exists() {
            return Err(Error::InvalidPath(format!(
                "{} does not exist inside the extracted archive",
                candidate.display()
            )));
        }
        Some(candidate)
    } else {
        find_main_executable(&dest, &name)
    };

    let mut desktop_file = None;
    let mut icon_path = None;
    let mut symlink_path = None;

    if let Some(exec) = &exec_path {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(exec)?.permissions();
            perms.set_mode(perms.mode() | 0o111);
            std::fs::set_permissions(exec, perms)?;
        }

        std::fs::create_dir_all(Paths::bin_dir())?;
        let link = Paths::bin_dir().join(&slug);
        let _ = std::fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink(exec, &link)?;
        symlink_path = Some(link.clone());

        let icon_str = if let Some(icon_src) = &opts.icon {
            Some(
                desktop::install_icon(icon_src, &slug)?
                    .to_string_lossy()
                    .to_string(),
            )
        } else {
            None
        };
        icon_path = icon_str.clone().map(PathBuf::from);
        desktop_file = Some(desktop::create_desktop_file(
            &slug,
            &name,
            &link,
            icon_str.as_deref(),
            false,
        )?);
    }

    let registry = Registry::open_default()?;
    registry.upsert_manual(&ManualInstall {
        id: slug.clone(),
        source: SourceId::Archive,
        name: name.clone(),
        version: String::new(),
        path: dest.to_string_lossy().to_string(),
        exec_path: symlink_path.map(|p| p.to_string_lossy().to_string()),
        desktop_file: desktop_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        icon_path: icon_path.map(|p| p.to_string_lossy().to_string()),
        installed_at: Utc::now(),
        source_url: None,
    })?;

    Ok(OperationOutcome::ok(
        format!("Extracted {name} to {}", dest.display()),
        format!(
            "extracted: {}\nlauncher: {}",
            dest.display(),
            exec_path
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "none (no single executable detected; pass --exec)".to_string())
        ),
    ))
}

pub fn remove_archive(id: &str) -> Result<OperationOutcome> {
    let registry = Registry::open_default()?;
    let Some(entry) = registry.remove_manual(id)? else {
        return Err(Error::NotFound(id.to_string()));
    };
    if let Some(desktop_file) = &entry.desktop_file {
        let _ = std::fs::remove_file(desktop_file);
    }
    if let Some(icon) = &entry.icon_path {
        let _ = std::fs::remove_file(icon);
    }
    if let Some(exec) = &entry.exec_path {
        let _ = std::fs::remove_file(exec);
    }
    if !entry.path.is_empty() {
        let _ = std::fs::remove_dir_all(&entry.path);
    }
    Ok(OperationOutcome::ok(
        format!("Removed {}", entry.name),
        format!("removed extracted directory {}", entry.path),
    ))
}

impl super::PackageSource for ArchiveSource {
    fn id(&self) -> SourceId {
        SourceId::Archive
    }

    fn is_available(&self) -> bool {
        true
    }

    fn list_installed(&self) -> Result<Vec<Package>> {
        let registry = Registry::open_default()?;
        Ok(registry
            .list_manual(Some(SourceId::Archive))?
            .into_iter()
            .map(|m| Package {
                id: m.id,
                name: m.name,
                version: m.version,
                description: format!("Manually installed archive at {}", m.path),
                source: Some(SourceId::Archive),
                installed: true,
                available_version: None,
                is_security_update: false,
                icon: m.icon_path,
            })
            .collect())
    }

    fn search(&self, _query: &str) -> Result<Vec<Package>> {
        Ok(Vec::new())
    }

    fn list_updates(&self) -> Result<Vec<UpdateInfo>> {
        // Archives have no remote index to diff against in v1.
        Ok(Vec::new())
    }

    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome> {
        match target {
            InstallTarget::LocalFile { path, .. } => {
                install_archive(path, &ArchiveInstallOptions::default())
            }
            other => Err(Error::UnsupportedInstallTarget(format!(
                "archive only installs a local file, got {other:?}"
            ))),
        }
    }

    fn remove(&self, id: &str) -> Result<OperationOutcome> {
        remove_archive(id)
    }

    fn update(&self, _id: Option<&str>) -> Result<OperationOutcome> {
        Err(Error::UnsupportedInstallTarget(
            "archives have no update mechanism; remove and reinstall the new version".to_string(),
        ))
    }
}
