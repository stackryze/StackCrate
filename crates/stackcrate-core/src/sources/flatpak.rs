//! Flatpak adapter. Shells out to the `flatpak` CLI using `--columns=` for
//! stable, tab-separated machine output wherever the subcommand supports it.

use crate::error::{Error, Result};
use crate::process::{self, CommandOutput, LineSink};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo, UpdateKind};
use std::collections::HashMap;
use std::time::Duration;

// Privilege note: unlike apt, a user-scoped `flatpak install/update` needs no
// root at all. System-wide Flatpak installs do need elevated rights, which
// Flatpak's own polkit helper (`flatpak-system-helper`) requests itself, so
// this adapter intentionally never wraps flatpak in pkexec/sudo.

pub struct FlatpakSource;

const TIMEOUT: Duration = Duration::from_secs(600);

pub fn is_available() -> bool {
    process::binary_available("flatpak")
}

pub fn list_installed() -> Result<Vec<Package>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let out = process::run(
        "flatpak",
        &[
            "list",
            "--app",
            "--columns=application,name,version,description,size",
        ],
        Duration::from_secs(30),
    )?;
    if !out.success() {
        return Ok(Vec::new());
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(parse_installed_line)
        .collect())
}

fn parse_installed_line(line: &str) -> Option<Package> {
    let mut cols = line.split('\t');
    let id = cols.next()?.trim();
    if id.is_empty() {
        return None;
    }
    let name = cols.next().unwrap_or(id).trim();
    let version = cols.next().unwrap_or("").trim();
    let description = cols.next().unwrap_or("").trim();
    let size_kb = cols
        .next()
        .and_then(|s| crate::types::parse_size_to_kb(s.trim()));
    Some(Package {
        id: id.to_string(),
        name: if name.is_empty() {
            id.to_string()
        } else {
            name.to_string()
        },
        version: version.to_string(),
        description: description.to_string(),
        source: Some(SourceId::Flatpak),
        installed: true,
        size_kb,
        ..Package::default()
    })
}

pub fn search(query: &str) -> Result<Vec<Package>> {
    process::require_binary("flatpak")?;
    let out = process::run(
        "flatpak",
        &[
            "search",
            "--columns=application,name,version,description",
            query,
        ],
        Duration::from_secs(30),
    )?;
    if !out.success() {
        if out.combined().to_ascii_lowercase().contains("no matches") {
            return Ok(Vec::new());
        }
        return Err(Error::CommandFailed {
            command: format!("flatpak search {query}"),
            code: out.code,
            output: out.combined(),
        });
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(parse_installed_line)
        .map(|mut p| {
            p.installed = false;
            p
        })
        .take(100)
        .collect())
}

fn remotes() -> Vec<String> {
    let Ok(out) = process::run(
        "flatpak",
        &["remotes", "--columns=name"],
        Duration::from_secs(15),
    ) else {
        return Vec::new();
    };
    if !out.success() {
        return Vec::new();
    }
    out.stdout
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect()
}

/// Best-effort pending-update listing: Flatpak has no single global
/// "--list" flag, so this checks `remote-ls --updates` per configured
/// remote. The displayed new-version string can lag up to ~24h behind
/// reality because it comes from cached appstream data (see
/// flatpak/flatpak#5974); the presence of an update itself is still
/// accurate, since it's based on installed-commit vs remote-summary state.
pub fn list_updates() -> Result<Vec<UpdateInfo>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let installed = list_installed()?;
    let current_versions: HashMap<String, String> = installed
        .iter()
        .map(|p| (p.id.clone(), p.version.clone()))
        .collect();

    let mut updates = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for remote in remotes() {
        let out = process::run(
            "flatpak",
            &[
                "remote-ls",
                &remote,
                "--app",
                "--updates",
                "--columns=application,version",
            ],
            Duration::from_secs(30),
        );
        let Ok(out) = out else { continue };
        if !out.success() {
            continue;
        }
        for line in out.stdout.lines() {
            let mut cols = line.split('\t');
            let Some(id) = cols.next().map(str::trim).filter(|s| !s.is_empty()) else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let new_version = cols.next().unwrap_or("").trim().to_string();
            updates.push(UpdateInfo {
                source: SourceId::Flatpak,
                id: id.to_string(),
                name: id.to_string(),
                current_version: current_versions.get(id).cloned().unwrap_or_default(),
                new_version,
                kind: UpdateKind::Normal,
            });
        }
    }
    Ok(updates)
}

pub fn install_named(name: &str) -> Result<OperationOutcome> {
    install_named_with(name, None)
}

pub fn install_named_with(name: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("flatpak")?;
    let out = run_maybe_streaming(
        &["install".to_string(), "-y".to_string(), name.to_string()],
        on_line.clone(),
    )?;
    if out.success() {
        return Ok(to_outcome(&format!("flatpak install {name}"), &out));
    }
    // Retry pinned to flathub if it exists; covers the common case where the
    // app is available from more than one configured remote.
    if remotes().iter().any(|r| r.eq_ignore_ascii_case("flathub")) {
        let retry = run_maybe_streaming(
            &[
                "install".to_string(),
                "-y".to_string(),
                "flathub".to_string(),
                name.to_string(),
            ],
            on_line,
        )?;
        return Ok(to_outcome(
            &format!("flatpak install flathub {name}"),
            &retry,
        ));
    }
    Ok(to_outcome(&format!("flatpak install {name}"), &out))
}

pub fn remove_named(id: &str) -> Result<OperationOutcome> {
    remove_named_with(id, None)
}

pub fn remove_named_with(id: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("flatpak")?;
    let out = run_maybe_streaming(
        &["uninstall".to_string(), "-y".to_string(), id.to_string()],
        on_line,
    )?;
    Ok(to_outcome(&format!("flatpak uninstall {id}"), &out))
}

pub fn update(id: Option<&str>) -> Result<OperationOutcome> {
    update_with(id, None)
}

pub fn update_with(id: Option<&str>, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("flatpak")?;
    let mut args = vec!["update".to_string(), "-y".to_string()];
    if let Some(id) = id {
        args.push(id.to_string());
    }
    let label = format!("flatpak {}", args.join(" "));
    let out = run_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&label, &out))
}

fn run_maybe_streaming(args: &[String], on_line: Option<LineSink>) -> Result<CommandOutput> {
    match on_line {
        Some(sink) => process::run_owned_streaming("flatpak", args, TIMEOUT, sink),
        None => process::run_owned("flatpak", args, TIMEOUT),
    }
}

/// Full details for one installed app, from `flatpak info`. Flatpak has no
/// equivalent for an app that isn't installed (no "show me this remote app's
/// metadata" subcommand with the same richness), so this only works for
/// already-installed ids.
pub fn details(id: &str) -> Result<Package> {
    process::require_binary("flatpak")?;
    let out = process::run("flatpak", &["info", id], Duration::from_secs(20))?;
    if !out.success() {
        return Err(Error::NotFound(id.to_string()));
    }
    let mut pkg = Package {
        id: id.to_string(),
        name: id.to_string(),
        source: Some(SourceId::Flatpak),
        installed: true,
        ..Package::default()
    };
    // First non-blank line is the human-readable title; everything else is
    // `Key: value` (aligned with spaces, not tabs, unlike `--columns`).
    let mut lines = out.stdout.lines();
    if let Some(title) = lines.next().map(str::trim).filter(|s| !s.is_empty()) {
        pkg.name = title.to_string();
    }
    for line in lines {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Version" => pkg.version = value.to_string(),
            "License" => pkg.license = Some(value.to_string()),
            "Installed" => pkg.size_kb = crate::types::parse_size_to_kb(value),
            "Origin" => pkg.description = format!("From {value}"),
            _ => {}
        }
    }
    Ok(pkg)
}

fn to_outcome(label: &str, out: &CommandOutput) -> OperationOutcome {
    if out.success() {
        OperationOutcome::ok(format!("{label} succeeded"), out.combined())
    } else {
        OperationOutcome::failed(
            format!("{label} failed (exit {})", out.code),
            out.combined(),
        )
    }
}

impl super::PackageSource for FlatpakSource {
    fn id(&self) -> SourceId {
        SourceId::Flatpak
    }

    fn is_available(&self) -> bool {
        is_available()
    }

    fn list_installed(&self) -> Result<Vec<Package>> {
        list_installed()
    }

    fn search(&self, query: &str) -> Result<Vec<Package>> {
        search(query)
    }

    fn list_updates(&self) -> Result<Vec<UpdateInfo>> {
        list_updates()
    }

    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome> {
        match target {
            InstallTarget::Named { name, .. } => install_named(name),
            other => Err(Error::UnsupportedInstallTarget(format!(
                "flatpak only installs named application ids, got {other:?}"
            ))),
        }
    }

    fn remove(&self, id: &str) -> Result<OperationOutcome> {
        remove_named(id)
    }

    fn update(&self, id: Option<&str>) -> Result<OperationOutcome> {
        update(id)
    }
}
