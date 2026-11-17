//! Snap adapter. Snap always needs root for install/remove/refresh, unlike
//! most of its own read-only queries (`list`, `find`, `refresh --list`).

use crate::error::{Error, Result};
use crate::privilege;
use crate::process::{self, CommandOutput, LineSink};
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
        source: Some(SourceId::Snap),
        installed: true,
        ..Package::default()
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
                ..Package::default()
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
    install_named_with(name, None)
}

pub fn install_named_with(name: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let args = vec!["install".to_string(), name.to_string()];
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&format!("snap install {name}"), &out))
}

pub fn remove_named(name: &str) -> Result<OperationOutcome> {
    remove_named_with(name, None)
}

pub fn remove_named_with(name: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let args = vec!["remove".to_string(), name.to_string()];
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&format!("snap remove {name}"), &out))
}

pub fn update(id: Option<&str>) -> Result<OperationOutcome> {
    update_with(id, None)
}

pub fn update_with(id: Option<&str>, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("snap")?;
    let mut args = vec!["refresh".to_string()];
    if let Some(name) = id {
        args.push(name.to_string());
    }
    let label = format!("snap {}", args.join(" "));
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&label, &out))
}

fn run_privileged_maybe_streaming(
    args: &[String],
    on_line: Option<LineSink>,
) -> Result<CommandOutput> {
    match on_line {
        Some(sink) => privilege::run_privileged_streaming("snap", args, TIMEOUT, sink),
        None => privilege::run_privileged("snap", args, TIMEOUT),
    }
}

/// Full details for one snap (installed or not) via `snap info`.
pub fn details(name: &str) -> Result<Package> {
    process::require_binary("snap")?;
    let out = process::run("snap", &["info", name], Duration::from_secs(20))?;
    if !out.success() {
        return Err(Error::NotFound(name.to_string()));
    }
    let mut pkg = Package {
        id: name.to_string(),
        name: name.to_string(),
        source: Some(SourceId::Snap),
        ..Package::default()
    };
    for line in out.stdout.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "summary" => pkg.description = value.to_string(),
            "publisher" => pkg.maintainer = Some(value.trim_end_matches('*').to_string()),
            "contact" => {
                if pkg.homepage.is_none() {
                    pkg.homepage = Some(value.to_string());
                }
            }
            "license" if value != "unset" => pkg.license = Some(value.to_string()),
            "installed" => {
                pkg.installed = true;
                // "installed:          1.0            (5) 4096B base"
                let fields: Vec<&str> = value.split_whitespace().collect();
                if let Some(version) = fields.first() {
                    pkg.version = version.to_string();
                }
                if let Some(size) = fields.iter().find(|f| {
                    f.ends_with('B')
                        && f.trim_end_matches(['B', 'K', 'M', 'G', 'i'])
                            .parse::<f64>()
                            .is_ok()
                }) {
                    pkg.size_kb = crate::types::parse_size_to_kb(size);
                }
            }
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
