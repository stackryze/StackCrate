//! Manual installs for AppImages. AppImages are self-contained, so "install"
//! just means: copy it somewhere stable, mark it executable, and (best
//! effort) pull its name/icon out via the bundled `--appimage-extract` so it
//! gets a proper application-menu entry instead of a bare file.

use crate::config::Paths;
use crate::desktop;
use crate::error::{Error, Result};
use crate::process;
use crate::registry::{ManualInstall, Registry};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo, UpdateKind};
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
    /// Recorded as-is in the registry's `version` column; the self-update
    /// path (`check_update`/`update`) fills this in with the release tag it
    /// just installed. Manual installs (CLI/GUI) leave it `None` since
    /// there's no reliable version signal for an arbitrary local file.
    pub version: Option<String>,
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
    let registry = Registry::open_default()?;
    if let Some(existing) = registry.get_manual(&slug)? {
        crate::snapshot::snapshot_if_present(&registry, &existing, &dest)?;
    } else if dest.exists() {
        std::fs::remove_file(&dest)?; // untracked leftover; nothing to snapshot
    }
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

    registry.upsert_manual(&ManualInstall {
        id: slug.clone(),
        source: SourceId::AppImage,
        name: name.clone(),
        version: opts.version.clone().unwrap_or_default(),
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

/// A parsed GitHub-releases `Updateinformation` string, e.g.
/// `gh-releases-zsync|owner|repo|latest|App-*.AppImage`. Only this scheme is
/// acted on automatically (by far the most common one in practice); a plain
/// `zsync|<url>` or direct static-URL scheme is recognized elsewhere as "has
/// update info" but isn't auto-checked yet (see ROADMAP.md).
struct GithubReleaseSource {
    owner: String,
    repo: String,
    release: String,
    filename_glob: String,
}

fn parse_update_info(raw: &str) -> Option<GithubReleaseSource> {
    let mut parts = raw.trim_matches('\0').trim().splitn(5, '|');
    if parts.next()? != "gh-releases-zsync" {
        return None;
    }
    Some(GithubReleaseSource {
        owner: parts.next()?.to_string(),
        repo: parts.next()?.to_string(),
        release: parts.next()?.to_string(),
        filename_glob: parts.next()?.to_string(),
    })
}

#[derive(serde::Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(serde::Deserialize, Clone)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    /// Added to GitHub's API relatively recently (format `"sha256:<hex>"`);
    /// not present on assets uploaded before GitHub computed it, so this is
    /// an opportunistic extra check, not something every release will have.
    digest: Option<String>,
}

fn fetch_latest_release(source: &GithubReleaseSource) -> Result<GithubRelease> {
    let url = if source.release == "latest" {
        format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            source.owner, source.repo
        )
    } else {
        format!(
            "https://api.github.com/repos/{}/{}/releases/tags/{}",
            source.owner, source.repo, source.release
        )
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("stackcrate/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let response = client.get(&url).send()?.error_for_status()?;
    let text = response.text()?;
    serde_json::from_str(&text)
        .map_err(|e| Error::other(format!("unexpected response from {url}: {e}")))
}

/// Simple `*`-wildcard glob match (AppImage filename globs never use
/// anything more exotic), implemented via the existing `regex` dependency
/// rather than adding a dedicated glob crate for one narrow need.
fn glob_matches(pattern: &str, candidate: &str) -> bool {
    let escaped: Vec<String> = pattern.split('*').map(regex::escape).collect();
    let pattern_re = format!("^{}$", escaped.join(".*"));
    regex::Regex::new(&pattern_re)
        .map(|re| re.is_match(candidate))
        .unwrap_or(false)
}

struct PendingUpdate {
    new_version: String,
    asset: GithubAsset,
}

/// Checks whether `path` (an installed AppImage of `installed_size` bytes)
/// has a newer release available, via its own embedded `.upd_info` section.
/// `None` covers every "nothing to do" case alike: no section, empty
/// section, an unrecognized/unsupported scheme, no matching asset, or the
/// matching asset being the same size as what's already installed (used as
/// the "different version" signal in lieu of semantic version comparison,
/// the same approach real-world AppImage update tooling falls back to when
/// a release doesn't carry a checksum).
fn check_update(path: &Path, installed_size: u64) -> Option<PendingUpdate> {
    let raw_bytes = crate::elf::read_section(path, ".upd_info")?;
    let raw = String::from_utf8_lossy(&raw_bytes);
    let source = parse_update_info(&raw)?;
    let release = fetch_latest_release(&source).ok()?;
    let asset = release
        .assets
        .into_iter()
        .find(|a| glob_matches(&source.filename_glob, &a.name))?;
    if asset.size == installed_size {
        return None;
    }
    Some(PendingUpdate {
        new_version: release.tag_name,
        asset,
    })
}

fn update_one(id: &str) -> Result<OperationOutcome> {
    let registry = Registry::open_default()?;
    let Some(entry) = registry.get_manual(id)? else {
        return Err(Error::NotFound(id.to_string()));
    };
    let path = PathBuf::from(&entry.path);
    let metadata = std::fs::metadata(&path)?;
    let Some(pending) = check_update(&path, metadata.len()) else {
        return Ok(OperationOutcome::ok(
            format!("{} is already up to date", entry.name),
            "",
        ));
    };

    // Unlike other adapters, this is the one place in the codebase that
    // needs to consult Config directly: it's about to install a file it
    // downloaded itself, with no CLI/GUI caller in a position to make this
    // call first (contrast with a URL the *user* gave `crate install`,
    // which the CLI checks before ever reaching an adapter).
    let config = crate::config::Config::load().unwrap_or_default();
    if config.require_checksum_for_appimage && pending.asset.digest.is_none() {
        return Ok(OperationOutcome::failed(
            format!(
                "{} has an update ({}), but config requires a checksum for AppImages and this \
                 release doesn't publish one; download and verify it manually",
                entry.name, pending.new_version
            ),
            pending.asset.browser_download_url,
        ));
    }

    let downloaded = crate::download::download(
        &pending.asset.browser_download_url,
        &Paths::cache_dir().join("downloads"),
    )?;
    if let Some(digest) = pending
        .asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        crate::verify::verify_checksum(&downloaded.path, digest)?;
    }

    let outcome = install_appimage(
        &downloaded.path,
        &AppImageInstallOptions {
            name: Some(entry.name.clone()),
            icon: None,
            no_launcher: false,
            version: Some(pending.new_version.clone()),
        },
    )?;
    Ok(OperationOutcome::ok(
        format!(
            "Updated {} to {} ({})",
            entry.name, pending.new_version, pending.asset.name
        ),
        outcome.log,
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
                size_kb: std::fs::metadata(&m.path).ok().map(|md| md.len() / 1024),
                icon: m.icon_path,
                ..Package::default()
            })
            .collect())
    }

    fn search(&self, _query: &str) -> Result<Vec<Package>> {
        Ok(Vec::new())
    }

    fn list_updates(&self) -> Result<Vec<UpdateInfo>> {
        let registry = Registry::open_default()?;
        let mut updates = Vec::new();
        for entry in registry.list_manual(Some(SourceId::AppImage))? {
            let path = PathBuf::from(&entry.path);
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let Some(pending) = check_update(&path, metadata.len()) else {
                continue;
            };
            updates.push(UpdateInfo {
                source: SourceId::AppImage,
                id: entry.id,
                name: entry.name,
                current_version: if entry.version.is_empty() {
                    "installed".to_string()
                } else {
                    entry.version
                },
                new_version: pending.new_version,
                kind: UpdateKind::Normal,
            });
        }
        Ok(updates)
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

    fn update(&self, id: Option<&str>) -> Result<OperationOutcome> {
        match id {
            Some(id) => update_one(id),
            None => Err(Error::UnsupportedInstallTarget(
                "AppImage updates are applied one at a time (no single 'update everything'); \
                 pass an id, e.g. `crate update <name> --source appimage`"
                    .to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gh_releases_zsync_scheme() {
        let source = parse_update_info("gh-releases-zsync|owner|repo|latest|App-*.AppImage\0\0\0")
            .expect("should parse");
        assert_eq!(source.owner, "owner");
        assert_eq!(source.repo, "repo");
        assert_eq!(source.release, "latest");
        assert_eq!(source.filename_glob, "App-*.AppImage");
    }

    #[test]
    fn rejects_unsupported_schemes() {
        assert!(parse_update_info("zsync|https://example.com/App.AppImage.zsync").is_none());
        assert!(parse_update_info("").is_none());
        assert!(parse_update_info("\0\0\0\0").is_none());
    }

    #[test]
    fn glob_matches_wildcard_filenames() {
        assert!(glob_matches("App-*.AppImage", "App-x86_64.AppImage"));
        assert!(glob_matches("*.AppImage", "anything.AppImage"));
        assert!(!glob_matches("App-*.AppImage", "Other-x86_64.AppImage"));
        assert!(!glob_matches("App-*.AppImage", "App-x86_64.zip"));
    }

    #[test]
    fn glob_matches_exact_filename_without_wildcard() {
        assert!(glob_matches("App.AppImage", "App.AppImage"));
        assert!(!glob_matches("App.AppImage", "App-other.AppImage"));
    }

    #[test]
    #[ignore = "hits the real GitHub API; not run by default, use --ignored to check manually"]
    fn live_github_api_check() {
        let source = GithubReleaseSource {
            owner: "AppImage".to_string(),
            repo: "AppImageUpdate".to_string(),
            release: "latest".to_string(),
            filename_glob: "AppImageUpdate-x86_64.AppImage".to_string(),
        };
        let release = fetch_latest_release(&source).expect("real API call should succeed");
        assert!(!release.tag_name.is_empty());
        let asset = release
            .assets
            .iter()
            .find(|a| glob_matches(&source.filename_glob, &a.name))
            .expect("expected a matching asset");
        assert!(asset.size > 0);
        assert!(asset.digest.as_deref().unwrap_or("").starts_with("sha256:"));
    }
}
