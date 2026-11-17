//! Adapter for Debian/Ubuntu's apt + dpkg. This is the primary system
//! package source for this build. `.deb` files are handled separately by
//! `deb_file.rs`, since installing a local file and installing a named
//! repository package use different commands.

use crate::error::{Error, Result};
use crate::privilege;
use crate::process::{self, CommandOutput};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo, UpdateKind};
use std::time::Duration;

/// Flags that keep `apt-get` fully non-interactive even when it would
/// otherwise prompt (e.g. a changed conffile). Needed because privilege
/// helpers (pkexec/sudo) don't reliably forward `DEBIAN_FRONTEND` through to
/// the child process, and because this adapter is also used for unattended
/// background updates.
const NONINTERACTIVE_APT_FLAGS: &[&str] = &[
    "-y",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

const TIMEOUT: Duration = Duration::from_secs(600);

pub struct AptSource;

pub fn is_available() -> bool {
    process::binary_available("apt-get")
        && process::binary_available("apt-cache")
        && process::binary_available("dpkg-query")
}

/// Refresh the local package index (`apt-get update`). Separate from
/// `list_updates` so callers control exactly when a privileged, networked
/// refresh happens versus a cheap local-only read.
pub fn refresh() -> Result<OperationOutcome> {
    process::require_binary("apt-get")?;
    let out =
        privilege::run_privileged("apt-get", &["update".to_string()], Duration::from_secs(180))?;
    Ok(to_outcome("apt-get update", &out))
}

pub fn list_installed() -> Result<Vec<Package>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let out = process::run(
        "dpkg-query",
        &[
            "-W",
            "-f=${binary:Package}\t${Version}\t${binary:Summary}\n",
        ],
        Duration::from_secs(60),
    );
    // Older dpkg-query doesn't know the `binary:Summary` field; retry without it.
    let out = match out {
        Ok(ref o) if o.success() => out,
        _ => process::run(
            "dpkg-query",
            &["-W", "-f=${binary:Package}\t${Version}\n"],
            Duration::from_secs(60),
        ),
    }?;
    if !out.success() {
        return Ok(Vec::new());
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let name = parts.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let version = parts.next().unwrap_or("").to_string();
            let description = parts.next().unwrap_or("").to_string();
            Some(Package {
                id: name.to_string(),
                name: name.to_string(),
                version,
                description,
                source: Some(SourceId::Apt),
                installed: true,
                available_version: None,
                is_security_update: false,
                icon: None,
            })
        })
        .collect())
}

pub fn search(query: &str) -> Result<Vec<Package>> {
    process::require_binary("apt-cache")?;
    let out = process::run(
        "apt-cache",
        &["search", "--names-only", query],
        Duration::from_secs(30),
    )?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("apt-cache search --names-only {query}"),
            code: out.code,
            output: out.combined(),
        });
    }
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| line.split_once(" - "))
        .take(200)
        .map(|(name, description)| Package {
            id: name.trim().to_string(),
            name: name.trim().to_string(),
            version: String::new(),
            description: description.trim().to_string(),
            source: Some(SourceId::Apt),
            installed: false,
            available_version: None,
            is_security_update: false,
            icon: None,
        })
        .collect())
}

/// Parse `apt list --upgradable` output. Reads only the local cache (no
/// network access), so this is safe to call as often as needed; call
/// `refresh()` first if you want it to reflect the latest repository state.
pub fn list_updates() -> Result<Vec<UpdateInfo>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    // `apt` (as opposed to `apt-cache`/`apt-get`) is the friendliest stable
    // way to get this particular listing; its CLI-stability warning is about
    // machine-parsing of general output, but `list --upgradable` has been
    // relied upon by sysadmin tooling for years and is what this project
    // documents as its parsing contract.
    process::require_binary("apt")?;
    let out = process::run("apt", &["list", "--upgradable"], Duration::from_secs(30))?;
    if !out.success() {
        return Ok(Vec::new());
    }
    let mut updates = Vec::new();
    for line in out.stdout.lines() {
        if line.starts_with("Listing...") || line.trim().is_empty() {
            continue;
        }
        if let Some(update) = parse_upgradable_line(line) {
            updates.push(update);
        }
    }
    Ok(updates)
}

fn parse_upgradable_line(line: &str) -> Option<UpdateInfo> {
    // Example:
    // firefox/noble-security,noble-security 129.0+build3-0ubuntu0.24.04.1 amd64 [upgradable from: 128.0+build2-0ubuntu0.24.04.1]
    let (name_and_suites, rest) = line.split_once(' ')?;
    let (name, suites) = name_and_suites.split_once('/')?;
    let new_version = rest.split_whitespace().next()?.to_string();
    let current_version = rest
        .split("upgradable from:")
        .nth(1)
        .map(|s| s.trim_end_matches(']').trim().to_string())
        .unwrap_or_default();
    let is_security_update = suites.to_ascii_lowercase().contains("security");
    Some(UpdateInfo {
        source: SourceId::Apt,
        id: name.to_string(),
        name: name.to_string(),
        current_version,
        new_version,
        kind: if is_security_update {
            UpdateKind::Security
        } else {
            UpdateKind::Normal
        },
    })
}

pub fn install_named(name: &str) -> Result<OperationOutcome> {
    crate::desktop::slugify(name).map_err(|_| Error::InvalidName(name.to_string()))?;
    process::require_binary("apt-get")?;
    let mut args = vec!["install".to_string()];
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    args.push(name.to_string());
    let out = privilege::run_privileged("apt-get", &args, TIMEOUT)?;
    Ok(to_outcome(&format!("apt-get install {name}"), &out))
}

pub fn remove_named(name: &str) -> Result<OperationOutcome> {
    process::require_binary("apt-get")?;
    let mut args = vec!["remove".to_string()];
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    args.push(name.to_string());
    let out = privilege::run_privileged("apt-get", &args, TIMEOUT)?;
    Ok(to_outcome(&format!("apt-get remove {name}"), &out))
}

pub fn update(id: Option<&str>) -> Result<OperationOutcome> {
    process::require_binary("apt-get")?;
    let mut args = vec![if id.is_some() {
        "install".to_string()
    } else {
        "upgrade".to_string()
    }];
    if id.is_some() {
        args.push("--only-upgrade".to_string());
    }
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    if let Some(name) = id {
        args.push(name.to_string());
    }
    let out = privilege::run_privileged("apt-get", &args, TIMEOUT)?;
    Ok(to_outcome(&format!("apt-get {}", args.join(" ")), &out))
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

impl super::PackageSource for AptSource {
    fn id(&self) -> SourceId {
        SourceId::Apt
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
                "apt only installs named repository packages, got {other:?}"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_security_upgrade_line() {
        let line = "firefox/noble-security,noble-security 129.0 amd64 [upgradable from: 128.0]";
        let update = parse_upgradable_line(line).unwrap();
        assert_eq!(update.name, "firefox");
        assert_eq!(update.new_version, "129.0");
        assert_eq!(update.current_version, "128.0");
        assert_eq!(update.kind, UpdateKind::Security);
    }

    #[test]
    fn parses_normal_upgrade_line() {
        let line = "vim/noble-updates 2:9.1.0-1ubuntu1 amd64 [upgradable from: 2:9.0.0-1ubuntu1]";
        let update = parse_upgradable_line(line).unwrap();
        assert_eq!(update.name, "vim");
        assert_eq!(update.kind, UpdateKind::Normal);
    }

    #[test]
    fn ignores_header_line() {
        assert!(parse_upgradable_line("Listing...").is_none());
    }

    #[test]
    fn ignores_garbage_line() {
        assert!(parse_upgradable_line("not a real line").is_none());
    }
}
