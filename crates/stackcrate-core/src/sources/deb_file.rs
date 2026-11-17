//! Installing a local `.deb` file directly (as opposed to a named package
//! from a configured repository, which is `apt.rs`'s job). Mirrors what
//! GDebi/`dpkg -i` + `apt-get -f install` have done for years: `dpkg -i`
//! unpacks the package, and if it has unmet dependencies it is left
//! half-configured; `apt-get install -f` then pulls in the missing
//! dependencies from the configured repositories and finishes configuring it.

use crate::error::{Error, Result};
use crate::privilege;
use crate::process::{self, CommandOutput, LineSink};
use crate::sources::apt;
use crate::types::{InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo};
use std::path::Path;
use std::time::Duration;

const NONINTERACTIVE_APT_FLAGS: &[&str] = &[
    "-y",
    "-o",
    "Dpkg::Options::=--force-confdef",
    "-o",
    "Dpkg::Options::=--force-confold",
];

pub struct DebFileSource;

#[derive(Debug, Clone, Default)]
pub struct DebMetadata {
    pub package: String,
    pub version: String,
    pub description: String,
    pub size_kb: Option<u64>,
    pub maintainer: Option<String>,
    pub homepage: Option<String>,
}

/// Read control metadata out of a `.deb` without installing it.
pub fn inspect(path: &Path) -> Result<DebMetadata> {
    process::require_binary("dpkg-deb")?;
    let out = process::run(
        "dpkg-deb",
        &["-I", &path.to_string_lossy()],
        Duration::from_secs(20),
    )?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("dpkg-deb -I {}", path.display()),
            code: out.code,
            output: out.combined(),
        });
    }
    let mut meta = DebMetadata::default();
    for line in out.stdout.lines() {
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix("Package:") {
            meta.package = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("Version:") {
            meta.version = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("Description:") {
            meta.description = value.trim().to_string();
        } else if let Some(value) = trimmed.strip_prefix("Installed-Size:") {
            meta.size_kb = value.trim().parse().ok();
        } else if let Some(value) = trimmed.strip_prefix("Maintainer:") {
            meta.maintainer = Some(value.trim().to_string());
        } else if let Some(value) = trimmed.strip_prefix("Homepage:") {
            meta.homepage = Some(value.trim().to_string());
        }
    }
    if meta.package.is_empty() {
        return Err(Error::other(format!(
            "could not read package metadata from {}",
            path.display()
        )));
    }
    Ok(meta)
}

pub fn install_file(path: &Path) -> Result<OperationOutcome> {
    install_file_with(path, None)
}

pub fn install_file_with(path: &Path, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    if !path.exists() {
        return Err(Error::InvalidPath(format!(
            "{} does not exist",
            path.display()
        )));
    }
    process::require_binary("dpkg")?;
    let meta = inspect(path)?;

    let dpkg_args = vec!["-i".to_string(), path.to_string_lossy().to_string()];
    let dpkg_out = match &on_line {
        Some(sink) => privilege::run_privileged_streaming(
            "dpkg",
            &dpkg_args,
            Duration::from_secs(300),
            sink.clone(),
        )?,
        None => privilege::run_privileged("dpkg", &dpkg_args, Duration::from_secs(300))?,
    };

    let mut log = format!("$ dpkg -i {}\n{}\n", path.display(), dpkg_out.combined());

    let final_out: CommandOutput = if dpkg_out.success() {
        dpkg_out
    } else {
        process::require_binary("apt-get")?;
        let mut args = vec!["install".to_string(), "-f".to_string()];
        args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
        let fix_out = match &on_line {
            Some(sink) => privilege::run_privileged_streaming(
                "apt-get",
                &args,
                Duration::from_secs(300),
                sink.clone(),
            )?,
            None => privilege::run_privileged("apt-get", &args, Duration::from_secs(300))?,
        };
        log.push_str(&format!(
            "$ apt-get install -f -y\n{}\n",
            fix_out.combined()
        ));
        fix_out
    };

    if final_out.success() {
        Ok(OperationOutcome::ok(
            format!(
                "Installed {} {} from {}",
                meta.package,
                meta.version,
                path.display()
            ),
            log,
        ))
    } else {
        Ok(OperationOutcome::failed(
            format!("Failed to install {}", path.display()),
            log,
        ))
    }
}

impl super::PackageSource for DebFileSource {
    fn id(&self) -> SourceId {
        SourceId::DebFile
    }

    fn is_available(&self) -> bool {
        process::binary_available("dpkg") && process::binary_available("apt-get")
    }

    fn list_installed(&self) -> Result<Vec<Package>> {
        // Once installed, a .deb is just a normal dpkg/apt-tracked package.
        Ok(Vec::new())
    }

    fn search(&self, _query: &str) -> Result<Vec<Package>> {
        Ok(Vec::new())
    }

    fn list_updates(&self) -> Result<Vec<UpdateInfo>> {
        Ok(Vec::new())
    }

    fn install(&self, target: &InstallTarget) -> Result<OperationOutcome> {
        match target {
            InstallTarget::LocalFile { path, .. } => install_file(path),
            other => Err(Error::UnsupportedInstallTarget(format!(
                "deb-file only installs a local .deb path, got {other:?}"
            ))),
        }
    }

    fn remove(&self, id: &str) -> Result<OperationOutcome> {
        apt::remove_named(id)
    }

    fn update(&self, id: Option<&str>) -> Result<OperationOutcome> {
        apt::update(id)
    }
}
