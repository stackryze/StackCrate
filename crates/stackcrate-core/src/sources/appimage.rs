//! Manual installs for AppImages. AppImages are self-contained, so "install"
//! just means: copy it somewhere stable, mark it executable, and (best
//! effort) pull its name/icon out via the bundled `--appimage-extract` so it
//! gets a proper application-menu entry instead of a bare file.

use crate::config::Paths;
use crate::desktop;
use crate::error::{Error, Result};
use crate::process;
use crate::registry::{ManualInstall, Registry};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub struct AppImageSource;

impl AppImageSource {
    pub fn new() -> Self {
        AppImageSource
    }
}

impl Default for AppImageSource {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ExtractedInfo {
    pub name: Option<String>,
    pub icon_path: Option<PathBuf>,
}

/// Run the AppImage's own `--appimage-extract` to pull out its `.desktop`
/// file and icon, without actually launching the application. Best-effort:
/// any failure here just means we fall back to a generic name/icon.
fn extract_desktop_info(appimage_path: &Path) -> Option<ExtractedInfo> {
    let tmp = tempfile::Builder::new()
        .prefix("stackcrate-appimage-")
        .tempdir()
        .ok()?;
    let out = process::run_in(
        &appimage_path.to_string_lossy(),
        &["--appimage-extract".to_string()],
        Some(tmp.path()),
        Duration::from_secs(60),
    )
    .ok()?;
    if !out.success() {
        return None;
    }
    let squashfs_root = tmp.path().join("squashfs-root");
    let desktop_file = std::fs::read_dir(&squashfs_root)
        .ok()?
        .flatten()
        .find(|e| e.path().extension().and_then(|x| x.to_str()) == Some("desktop"))?
        .path();
    let contents = std::fs::read_to_string(&desktop_file).ok()?;
    let mut name = None;
    let mut icon_name = None;
    for line in contents.lines() {
        if let Some(v) = line.strip_prefix("Name=") {
            if name.is_none() {
                name = Some(v.trim().to_string());
            }
        } else if let Some(v) = line.strip_prefix("Icon=")
            && icon_name.is_none()
        {
            icon_name = Some(v.trim().to_string());
        }
    }
    let icon_path = icon_name.and_then(|icon| {
        for ext in ["png", "svg", "xpm"] {
            let candidate = squashfs_root.join(format!("{icon}.{ext}"));
            if candidate.exists() {
                return persist_icon(&candidate);
            }
        }
        let dir_icon = squashfs_root.join(".DirIcon");
        if dir_icon.exists() {
            return persist_icon(&dir_icon);
        }
        None
    });
    Some(ExtractedInfo { name, icon_path })
}

/// Copy an icon out of the (about to be deleted) temp extraction dir into a
/// stable location before the tempdir is cleaned up.
fn persist_icon(path: &Path) -> Option<PathBuf> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("png");
    let dest = std::env::temp_dir().join(format!(
        "stackcrate-appimage-icon-{}.{ext}",
        uuid::Uuid::new_v4()
    ));
    std::fs::copy(path, &dest).ok()?;
    Some(dest)
}

fn default_name_from_path(path: &Path) -> String {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("app");
    stem.trim_end_matches(".AppImage")
        .trim_end_matches(".appimage")
        .to_string()
}

#[derive(Debug, Clone, Default)]
pub struct AppImageInstallOptions {
    pub name: Option<String>,
    pub icon: Option<PathBuf>,
    pub no_launcher: bool,
}

pub fn install_appimage(path: &Path, opts: &AppImageInstallOptions) -> Result<OperationOutcome> {
    if !path.exists() {
        return Err(Error::InvalidPath(format!(
            "{} does not exist",
            path.display()
        )));
    }
    let extracted = extract_desktop_info(path);
    let name = opts
        .name
        .clone()
        .or_else(|| extracted.as_ref().and_then(|e| e.name.clone()))
        .unwrap_or_else(|| default_name_from_path(path));
    let slug = desktop::slugify(&name)?;

    std::fs::create_dir_all(Paths::appimage_dir())?;
    let dest = Paths::appimage_dir().join(format!("{slug}.AppImage"));
    std::fs::copy(path, &dest)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&dest)?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(&dest, perms)?;
    }

    let mut desktop_file = None;
    let mut icon_path = None;
    if !opts.no_launcher {
        let icon_source = opts
            .icon
            .clone()
            .or_else(|| extracted.and_then(|e| e.icon_path));
        let icon_str = if let Some(icon_src) = &icon_source {
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
            &dest,
            icon_str.as_deref(),
            false,
        )?);
    }

    let registry = Registry::open_default()?;
    registry.upsert_manual(&ManualInstall {
        id: slug.clone(),
        source: SourceId::AppImage,
        name: name.clone(),
        version: String::new(),
        path: dest.to_string_lossy().to_string(),
        exec_path: Some(dest.to_string_lossy().to_string()),
        desktop_file: desktop_file
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
        icon_path: icon_path.map(|p| p.to_string_lossy().to_string()),
        installed_at: Utc::now(),
        source_url: None,
    })?;

    Ok(OperationOutcome::ok(
        format!("Installed {name} to {}", dest.display()),
        format!("appimage: {}", dest.display()),
    ))
}

pub fn remove_appimage(id: &str) -> Result<OperationOutcome> {
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
    if !entry.path.is_empty() {
        let _ = std::fs::remove_file(&entry.path);
    }
    Ok(OperationOutcome::ok(
        format!("Removed {}", entry.name),
        format!("removed {}", entry.path),
    ))
}

impl super::PackageSource for AppImageSource {
    fn id(&self) -> SourceId {
        SourceId::AppImage
    }

    fn is_available(&self) -> bool {
        true
    }

    fn list_installed(&self) -> Result<Vec<Package>> {
        let registry = Registry::open_default()?;
        Ok(registry
            .list_manual(Some(SourceId::AppImage))?
            .into_iter()
            .map(|m| Package {
                id: m.id,
                name: m.name,
                version: m.version,
                description: format!("AppImage at {}", m.path),
                source: Some(SourceId::AppImage),
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
        // Phase 2: parse embedded "Updateinformation" (zsync) when present.
        Ok(Vec::new())
    }

    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome> {
        match target {
            InstallTarget::LocalFile { path, .. } => {
                install_appimage(path, &AppImageInstallOptions::default())
            }
            other => Err(Error::UnsupportedInstallTarget(format!(
                "appimage only installs a local file, got {other:?}"
            ))),
        }
    }

    fn remove(&self, id: &str) -> Result<OperationOutcome> {
        remove_appimage(id)
    }

    fn update(&self, _id: Option<&str>) -> Result<OperationOutcome> {
        Err(Error::UnsupportedInstallTarget(
            "AppImage auto-update is not yet implemented; remove and reinstall the new version"
                .to_string(),
        ))
    }
}
