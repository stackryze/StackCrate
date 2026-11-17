//! Snap adapter. Snap always needs root for install/remove/refresh, unlike
//! most of its own read-only queries (`list`, `find`, `refresh --list`).

use crate::error::{Error, Result};
use crate::privilege;
use crate::process::{self, CommandOutput};
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo, UpdateKind};
use std::collections::HashMap;
use std::time::Duration;

pub struct SnapSource;

const TIMEOUT: Duration = Duration::from_secs(600);

pub fn is_available() -> bool {
    process::binary_available("snap")
}

pub fn list_installed() -> Result<Vec<Package>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let out = process::run("snap", &["list"], Duration::from_secs(20))?;
    if !out.success() {
        return Ok(Vec::new());
    }
    Ok(out
        .stdout
        .lines()
        .skip(1)
        .filter_map(parse_list_line)
        .collect())
}

fn parse_list_line(line: &str) -> Option<Package> {
    let mut parts = line.split_whitespace();
    let name = parts.next()?;
    let version = parts.next().unwrap_or("").to_string();
    Some(Package {
        id: name.to_string(),
        name: name.to_string(),
        version,
        description: String::new(),
        source: Some(SourceId::Snap),
        installed: true,
        available_version: None,
        is_security_update: false,
        icon: None,
    })
}

pub fn search(query: &str) -> Result<Vec<Package>> {
    process::require_binary("snap")?;
    let out = process::run("snap", &["find", query], Duration::from_secs(30))?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("snap find {query}"),
            code: out.code,
            output: out.combined(),
        });
    }
    let column_gap = regex::Regex::new(r"\s{2,}").expect("static regex is valid");
    Ok(out
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = column_gap.split(line.trim()).collect();
            let name = *cols.first()?;
            if name.is_empty() {
                return None;
            }
            Some(Package {
                id: name.to_string(),
                name: name.to_string(),
                version: cols.get(1).copied().unwrap_or("").to_string(),
                description: cols.get(4).copied().unwrap_or("").to_string(),
                source: Some(SourceId::Snap),
                installed: false,
                available_version: None,
                is_security_update: false,
                icon: None,
            })
        })
        .take(100)
        .collect())
}

/// `snap refresh --list` needs no privilege and never changes anything; it
/// lists the new revision available for each snap with a pending update.
pub fn list_updates() -> Result<Vec<UpdateInfo>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let out = process::run("snap", &["refresh", "--list"], Duration::from_secs(30))?;
    if !out.success() {
        // Exit code is non-zero (1) when there is simply nothing to refresh.
        return Ok(Vec::new());
    }
    let installed: HashMap<String, String> = list_installed()?
        .into_iter()
        .map(|p| (p.id, p.version))
        .collect();
    Ok(out
        .stdout
        .lines()
        .skip(1)
        .filter_map(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            let name = *cols.first()?;
            let new_version = cols.get(1).copied().unwrap_or("").to_string();
            Some(UpdateInfo {
                source: SourceId::Snap,
                id: name.to_string(),
                name: name.to_string(),
                current_version: installed.get(name).cloned().unwrap_or_default(),
                new_version,
                kind: UpdateKind::Normal,
            })
        })
        .collect())
}

pub fn install_named(name: &str) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let out =
        privilege::run_privileged("snap", &["install".to_string(), name.to_string()], TIMEOUT)?;
    Ok(to_outcome(&format!("snap install {name}"), &out))
}

pub fn remove_named(name: &str) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let out =
        privilege::run_privileged("snap", &["remove".to_string(), name.to_string()], TIMEOUT)?;
    Ok(to_outcome(&format!("snap remove {name}"), &out))
}

pub fn update(id: Option<&str>) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let mut args = vec!["refresh".to_string()];
    if let Some(name) = id {
        args.push(name.to_string());
    }
    let out = privilege::run_privileged("snap", &args, TIMEOUT)?;
    Ok(to_outcome(&format!("snap {}", args.join(" ")), &out))
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

impl super::PackageSource for SnapSource {
    fn id(&self) -> SourceId {
        SourceId::Snap
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
                "snap only installs named packages, got {other:?}"
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
