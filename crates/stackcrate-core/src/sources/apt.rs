//! Adapter for Debian/Ubuntu's apt + dpkg. This is the primary system
//! package source for this build. `.deb` files are handled separately by
//! `deb_file.rs`, since installing a local file and installing a named
//! repository package use different commands.

use crate::error::{Error, Result};
use crate::privilege;
use crate::process::{self, CommandOutput, LineSink};
use crate::types::{
    ExplainInfo, InstallTarget, OperationOutcome, Package, SourceId, UpdateInfo, UpdateKind,
};
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
    // `${Installed-Size}` is reported in KiB (dpkg's native unit) and has
    // been a stable field since forever, so it's included unconditionally;
    // `${binary:Summary}` is newer and falls back below on older dpkg-query.
    let out = process::run(
        "dpkg-query",
        &[
            "-W",
            "-f=${binary:Package}\t${Version}\t${Installed-Size}\t${binary:Summary}\n",
        ],
        Duration::from_secs(60),
    );
    let out = match out {
        Ok(ref o) if o.success() => out,
        _ => process::run(
            "dpkg-query",
            &[
                "-W",
                "-f=${binary:Package}\t${Version}\t${Installed-Size}\n",
            ],
            Duration::from_secs(60),
        ),
    }?;
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
    let mut parts = line.split('\t');
    let name = parts.next()?.trim();
    if name.is_empty() {
        return None;
    }
    let version = parts.next().unwrap_or("").to_string();
    let size_kb = parts.next().and_then(|s| s.trim().parse().ok());
    let description = parts.next().unwrap_or("").to_string();
    Some(Package {
        id: name.to_string(),
        name: name.to_string(),
        version,
        description,
        source: Some(SourceId::Apt),
        installed: true,
        size_kb,
        ..Package::default()
    })
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
            description: description.trim().to_string(),
            source: Some(SourceId::Apt),
            ..Package::default()
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
    install_named_with(name, None)
}

pub fn install_named_with(name: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    crate::desktop::slugify(name).map_err(|_| Error::InvalidName(name.to_string()))?;
    process::require_binary("apt-get")?;
    let mut args = vec!["install".to_string()];
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    args.push(name.to_string());
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&format!("apt-get install {name}"), &out))
}

pub fn remove_named(name: &str) -> Result<OperationOutcome> {
    remove_named_with(name, None)
}

pub fn remove_named_with(name: &str, on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("apt-get")?;
    let mut args = vec!["remove".to_string()];
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    args.push(name.to_string());
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&format!("apt-get remove {name}"), &out))
}

pub fn update(id: Option<&str>) -> Result<OperationOutcome> {
    update_with(id, None)
}

pub fn update_with(id: Option<&str>, on_line: Option<LineSink>) -> Result<OperationOutcome> {
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
    let label = format!("apt-get {}", args.join(" "));
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome(&label, &out))
}

/// Packages apt would remove as no-longer-needed dependencies
/// (`apt-get autoremove`'s own candidate list), as a read-only simulation
/// that needs no privilege escalation.
pub fn list_autoremovable() -> Result<Vec<Package>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let Ok(out) = process::run("apt-get", &["-s", "autoremove"], Duration::from_secs(60)) else {
        return Ok(Vec::new());
    };
    let names = parse_remv_names(&out.stdout);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    Ok(list_installed()?
        .into_iter()
        .filter(|p| names.contains(&p.id))
        .collect())
}

pub fn autoremove() -> Result<OperationOutcome> {
    autoremove_with(None)
}

pub fn autoremove_with(on_line: Option<LineSink>) -> Result<OperationOutcome> {
    process::require_binary("apt-get")?;
    let mut args = vec!["autoremove".to_string()];
    args.extend(NONINTERACTIVE_APT_FLAGS.iter().map(|s| s.to_string()));
    let out = run_privileged_maybe_streaming(&args, on_line)?;
    Ok(to_outcome("apt-get autoremove", &out))
}

/// What else would be removed along with `name` (reverse dependents that
/// have no other provider), e.g. to warn before a confirmation prompt.
/// Read-only simulation; `name` itself is excluded from the result.
pub fn removal_preview(name: &str) -> Result<Vec<String>> {
    if !is_available() {
        return Ok(Vec::new());
    }
    let Ok(out) = process::run("apt-get", &["-s", "remove", name], Duration::from_secs(30)) else {
        return Ok(Vec::new());
    };
    let mut others: Vec<String> = parse_remv_names(&out.stdout)
        .into_iter()
        .filter(|n| n != name)
        .collect();
    others.sort();
    Ok(others)
}

fn parse_remv_names(simulate_output: &str) -> std::collections::BTreeSet<String> {
    simulate_output
        .lines()
        .filter_map(|l| l.strip_prefix("Remv "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|s| s.to_string())
        .collect()
}

/// Hold `name` at its current version (`apt-mark hold`): both StackCrate's
/// own bulk-update and a plain `apt upgrade` run outside StackCrate will
/// skip it until `unhold` is called.
pub fn hold(name: &str) -> Result<OperationOutcome> {
    process::require_binary("apt-mark")?;
    let out = privilege::run_privileged(
        "apt-mark",
        &["hold".to_string(), name.to_string()],
        Duration::from_secs(30),
    )?;
    Ok(to_outcome(&format!("apt-mark hold {name}"), &out))
}

pub fn unhold(name: &str) -> Result<OperationOutcome> {
    process::require_binary("apt-mark")?;
    let out = privilege::run_privileged(
        "apt-mark",
        &["unhold".to_string(), name.to_string()],
        Duration::from_secs(30),
    )?;
    Ok(to_outcome(&format!("apt-mark unhold {name}"), &out))
}

/// Full details for one package: candidate metadata from `apt-cache show`,
/// reconciled with whether (and at what version) it's actually installed.
/// Unlike `search`/`list_installed`, this is one extra process spawn per
/// call, so it's meant for on-demand use (a detail view, `crate show`), not
/// for enriching bulk listings.
pub fn details(name: &str) -> Result<Package> {
    process::require_binary("apt-cache")?;
    let show = process::run("apt-cache", &["show", name], Duration::from_secs(20))?;
    if !show.success() || show.stdout.trim().is_empty() {
        return Err(Error::NotFound(name.to_string()));
    }
    let stanza = show.stdout.split("\n\n").next().unwrap_or(&show.stdout);

    let mut pkg = Package {
        id: name.to_string(),
        name: name.to_string(),
        source: Some(SourceId::Apt),
        ..Package::default()
    };
    let mut description: Option<String> = None;
    let mut in_description = false;
    for line in stanza.lines() {
        if let Some(v) = line
            .strip_prefix("Description:")
            .or_else(|| line.strip_prefix("Description-en:"))
        {
            description.get_or_insert_with(|| v.trim().to_string());
            in_description = true;
            continue;
        }
        if in_description && line.starts_with(' ') {
            continue; // wrapped continuation line of the long description
        }
        in_description = false;
        if let Some(v) = line.strip_prefix("Version:") {
            pkg.version = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("Installed-Size:") {
            pkg.size_kb = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("Maintainer:") {
            pkg.maintainer = Some(v.trim().to_string());
        } else if let Some(v) = line.strip_prefix("Homepage:") {
            pkg.homepage = Some(v.trim().to_string());
        }
    }
    pkg.description = description.unwrap_or_default();
    pkg.license = local_copyright_license(name);

    let candidate_version = pkg.version.clone();
    let installed_version = process::run(
        "dpkg-query",
        &["-W", "-f=${Version}", name],
        Duration::from_secs(10),
    )
    .ok()
    .filter(|o| o.success())
    .map(|o| o.stdout.trim().to_string())
    .filter(|s| !s.is_empty());
    if let Some(installed_version) = installed_version {
        pkg.installed = true;
        pkg.version = installed_version;
        if !candidate_version.is_empty() && candidate_version != pkg.version {
            pkg.available_version = Some(candidate_version);
        }
        pkg.manually_installed =
            process::run("apt-mark", &["showmanual", name], Duration::from_secs(10))
                .ok()
                .map(|out| out.stdout.lines().any(|l| l.trim() == name));
    }
    Ok(pkg)
}

/// Best-effort one-line license summary read straight from the installed
/// package's `/usr/share/doc/<pkg>/copyright` (DEP-5 machine-readable
/// format uses a `License:` field); Debian packages don't carry license
/// metadata in `apt-cache show` itself the way Flatpak/Snap do.
fn local_copyright_license(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(format!("/usr/share/doc/{name}/copyright")).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("License:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Why `name` is installed: whether it was explicitly requested versus
/// pulled in as a dependency, and what else currently depends on it.
pub fn explain(name: &str) -> Result<ExplainInfo> {
    let manually_installed =
        process::run("apt-mark", &["showmanual", name], Duration::from_secs(10))
            .ok()
            .map(|out| out.stdout.lines().any(|l| l.trim() == name));
    let required_by = process::run(
        "apt-cache",
        &["rdepends", "--installed", name],
        Duration::from_secs(20),
    )
    .ok()
    .map(|out| parse_rdepends(&out.stdout))
    .unwrap_or_default();
    Ok(ExplainInfo {
        manually_installed,
        required_by,
    })
}

fn parse_rdepends(output: &str) -> Vec<String> {
    let mut names: Vec<String> = output
        .lines()
        .skip_while(|l| !l.trim().eq_ignore_ascii_case("reverse depends:"))
        .skip(1)
        .map(|l| l.trim().trim_start_matches('|').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// `apt-get changelog`'s upstream/Debian changelog for `name`. Needs network
/// access to the configured archive; falls back to the locally shipped copy
/// (if the package is installed and ships one) when that fails.
pub fn changelog(name: &str) -> Result<String> {
    process::require_binary("apt-get")?;
    if let Ok(out) = process::run("apt-get", &["changelog", name], Duration::from_secs(30))
        && out.success()
        && !out.stdout.trim().is_empty()
    {
        return Ok(out.stdout);
    }
    for candidate in [
        format!("/usr/share/doc/{name}/changelog.Debian.gz"),
        format!("/usr/share/doc/{name}/changelog.gz"),
    ] {
        if let Ok(text) = read_gzip_text(&candidate) {
            return Ok(text);
        }
    }
    Err(Error::other(format!(
        "no changelog available for {name} (needs network access to fetch it, or a local /usr/share/doc/{name}/changelog*.gz)"
    )))
}

fn read_gzip_text(path: &str) -> std::io::Result<String> {
    use flate2::read::GzDecoder;
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut decoder = GzDecoder::new(file);
    let mut text = String::new();
    decoder.read_to_string(&mut text)?;
    Ok(text)
}

fn run_privileged_maybe_streaming(
    args: &[String],
    on_line: Option<LineSink>,
) -> Result<CommandOutput> {
    match on_line {
        Some(sink) => privilege::run_privileged_streaming("apt-get", args, TIMEOUT, sink),
        None => privilege::run_privileged("apt-get", args, TIMEOUT),
    }
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

    #[test]
    fn parses_installed_line_with_size_and_summary() {
        let pkg = parse_installed_line("vim\t2:9.1.0-1ubuntu1\t3456\tVi IMproved").unwrap();
        assert_eq!(pkg.name, "vim");
        assert_eq!(pkg.size_kb, Some(3456));
        assert_eq!(pkg.description, "Vi IMproved");
        assert!(pkg.installed);
    }

    #[test]
    fn parses_installed_line_without_summary_fallback() {
        let pkg = parse_installed_line("vim\t2:9.1.0-1ubuntu1\t3456").unwrap();
        assert_eq!(pkg.size_kb, Some(3456));
        assert_eq!(pkg.description, "");
    }

    #[test]
    fn parses_remv_names_from_simulate_output() {
        let output = "Remv firefox [128.0]\nRemv libfoo [1.2]\nConf something\n";
        let names = parse_remv_names(output);
        assert!(names.contains("firefox"));
        assert!(names.contains("libfoo"));
        assert!(!names.contains("something"));
    }

    #[test]
    fn parses_rdepends_output() {
        let output = "vim\nReverse Depends:\n  vim-gtk3\n  |some-meta-package\n";
        let names = parse_rdepends(output);
        assert_eq!(
            names,
            vec!["some-meta-package".to_string(), "vim-gtk3".to_string()]
        );
    }
}
