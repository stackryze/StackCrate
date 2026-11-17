use crate::cli::{ConfigCommand, ListCommand, SetupCommand};
use crate::output;
use anyhow::{Context, Result, anyhow, bail};
use colored::Colorize;
use pallet_core::Engine;
use pallet_core::config::{Config, UpdatePolicy};
use pallet_core::registry::HistoryEntry;
use pallet_core::sources::{
    self, appimage::AppImageInstallOptions, archive::ArchiveInstallOptions,
};
use pallet_core::types::{InstallTarget, OperationOutcome, SourceId};
use std::path::PathBuf;
use std::time::Duration;

const IPC_TIMEOUT: Duration = Duration::from_secs(10);

pub fn search(engine: &Engine, query: &str, source: Option<SourceId>) -> Result<()> {
    match source {
        Some(id) => {
            let results = sources::source_by_id(id)
                .search(query)
                .with_context(|| format!("searching {id}"))?;
            output::print_packages(&results);
        }
        None => {
            for (id, result) in engine.search_all(query) {
                match result {
                    Ok(pkgs) if !pkgs.is_empty() => output::print_packages(&pkgs),
                    Ok(_) => {}
                    Err(e) => output::print_failure(&format!("{id}: {e}")),
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn install(
    engine: &Engine,
    target: &str,
    source_hint: Option<SourceId>,
    name: Option<String>,
    exec: Option<String>,
    icon: Option<PathBuf>,
    no_launcher: bool,
    checksum: Option<String>,
    assume_yes: bool,
) -> Result<()> {
    let resolved = sources::resolve_install_target(target, source_hint)?;
    let resolved = materialize(engine, resolved, checksum.as_deref())?;

    let description = describe_target(&resolved);
    if !output::confirm(&format!("Install {description}?"), assume_yes) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }

    let source_id = resolved.source();
    let outcome = match &resolved {
        InstallTarget::LocalFile {
            path,
            source: SourceId::Archive,
        } => {
            let opts = ArchiveInstallOptions {
                name,
                exec_relative: exec,
                icon,
                no_launcher,
            };
            sources::archive::install_archive(path, &opts)?
        }
        InstallTarget::LocalFile {
            path,
            source: SourceId::AppImage,
        } => {
            let opts = AppImageInstallOptions {
                name,
                icon,
                no_launcher,
            };
            sources::appimage::install_appimage(path, &opts)?
        }
        other => sources::source_by_id(source_id).install(other)?,
    };

    report(&outcome);
    log_history(
        engine,
        source_id,
        "install",
        &describe_target(&resolved),
        &outcome,
    )?;
    if !outcome.success {
        bail!("install failed");
    }
    Ok(())
}

/// If the resolved target is a URL, download it, verify a checksum when one
/// is required or supplied, and re-sniff the downloaded file (the URL-based
/// guess is best-effort only).
fn materialize(
    engine: &Engine,
    target: InstallTarget,
    checksum: Option<&str>,
) -> Result<InstallTarget> {
    let InstallTarget::Url { url, .. } = &target else {
        return Ok(target);
    };
    println!("{} {url}", "Downloading".bold());
    let cache_dir = pallet_core::config::Paths::cache_dir().join("downloads");
    let downloaded = pallet_core::download::download(url, &cache_dir)
        .with_context(|| format!("downloading {url}"))?;

    let kind = sources::sniff::detect(&downloaded.path)?.ok_or_else(|| {
        anyhow!("downloaded file's type could not be determined; pass --source explicitly")
    })?;

    let needs_checksum = match kind {
        SourceId::AppImage => engine.config.require_checksum_for_appimage,
        SourceId::Archive => engine.config.require_checksum_for_archive,
        _ => false,
    };
    if let Some(expected) = checksum {
        pallet_core::verify::verify_checksum(&downloaded.path, expected)
            .context("checksum verification failed")?;
        println!("{}", "Checksum verified.".green());
    } else if needs_checksum {
        bail!("config requires a checksum for this source; re-run with --checksum <sha256|sha512>");
    }

    Ok(InstallTarget::LocalFile {
        source: kind,
        path: downloaded.path,
    })
}

fn describe_target(target: &InstallTarget) -> String {
    match target {
        InstallTarget::Named { name, source } => format!("{name} (via {source})"),
        InstallTarget::LocalFile { path, source } => format!("{} (via {source})", path.display()),
        InstallTarget::Url { url, source } => format!("{url} (via {source})"),
    }
}

pub fn remove(engine: &Engine, id: &str, source: Option<SourceId>, assume_yes: bool) -> Result<()> {
    let source_id = match source {
        Some(s) => s,
        None => find_owning_source(engine, id)?,
    };
    if !output::confirm(&format!("Remove {id} ({source_id})?"), assume_yes) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let outcome = sources::source_by_id(source_id).remove(id)?;
    report(&outcome);
    log_history(engine, source_id, "remove", id, &outcome)?;
    if !outcome.success {
        bail!("remove failed");
    }
    Ok(())
}

pub fn update(
    engine: &Engine,
    id: Option<String>,
    source: Option<SourceId>,
    refresh: bool,
    assume_yes: bool,
) -> Result<()> {
    if refresh {
        for outcome in engine.refresh_metadata()? {
            report(&outcome);
        }
    }

    if let Some(id) = &id {
        let source_id = match source {
            Some(s) => s,
            None => find_owning_source(engine, id)?,
        };
        if !output::confirm(&format!("Update {id} ({source_id})?"), assume_yes) {
            println!("{}", "Aborted.".dimmed());
            return Ok(());
        }
        let outcome = sources::source_by_id(source_id).update(Some(id))?;
        report(&outcome);
        log_history(engine, source_id, "update", id, &outcome)?;
        if !outcome.success {
            bail!("update failed");
        }
        return Ok(());
    }

    let targets: Vec<SourceId> = match source {
        Some(s) => vec![s],
        None => engine.active_sources().iter().map(|s| s.id()).collect(),
    };
    if !output::confirm(
        &format!(
            "Update everything on: {}?",
            targets
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        assume_yes,
    ) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let mut any_failed = false;
    for source_id in targets {
        let adapter = sources::source_by_id(source_id);
        match adapter.update(None) {
            Ok(outcome) => {
                report(&outcome);
                log_history(engine, source_id, "update", "(all)", &outcome)?;
                any_failed |= !outcome.success;
            }
            Err(e) => {
                output::print_failure(&format!("{source_id}: {e}"));
                any_failed = true;
            }
        }
    }
    if any_failed {
        bail!("one or more updates failed");
    }
    Ok(())
}

pub fn list(engine: &Engine, what: &ListCommand) -> Result<()> {
    match what {
        ListCommand::Installed { source } => {
            let sources_to_query: Vec<Box<dyn sources::PackageSource>> = match source {
                Some(id) => vec![sources::source_by_id(*id)],
                None => engine.active_sources(),
            };
            for adapter in sources_to_query {
                match adapter.list_installed() {
                    Ok(pkgs) => output::print_packages(&pkgs),
                    Err(e) => output::print_failure(&format!("{}: {e}", adapter.id())),
                }
            }
        }
        ListCommand::Updates => {
            output::print_updates(&engine.registry.cached_updates()?);
        }
        ListCommand::Sources => {
            for source in SourceId::all() {
                let adapter = sources::source_by_id(*source);
                let available = adapter.is_available();
                let enabled = engine.config.is_source_enabled(*source);
                let mark = if available && enabled {
                    "available".green()
                } else if !enabled {
                    "disabled".dimmed()
                } else {
                    "not installed".yellow()
                };
                println!("{:<10} {}", source.as_str(), mark);
            }
        }
    }
    Ok(())
}

pub fn status(engine: &Engine) -> Result<()> {
    match pallet_core::ipc::request_blocking(&pallet_core::ipc::Request::Status, IPC_TIMEOUT) {
        Ok(pallet_core::ipc::Response::Status {
            last_checked_at,
            update_count,
            security_count,
        }) => {
            print_status(last_checked_at, update_count, security_count, true);
        }
        _ => {
            let updates = engine.registry.cached_updates()?;
            let security = updates
                .iter()
                .filter(|u| u.kind == pallet_core::types::UpdateKind::Security)
                .count();
            print_status(
                engine.registry.last_checked_at()?,
                updates.len(),
                security,
                false,
            );
        }
    }
    Ok(())
}

fn print_status(
    last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    update_count: usize,
    security_count: usize,
    daemon_running: bool,
) {
    println!(
        "daemon:        {}",
        if daemon_running {
            "running".green()
        } else {
            "not running".yellow()
        }
    );
    println!(
        "last checked:  {}",
        last_checked_at
            .map(|t| t
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string())
            .unwrap_or_else(|| "never".to_string())
    );
    println!("updates:       {update_count} ({security_count} security)");
    if !daemon_running {
        println!(
            "{}",
            "Start the background monitor with: systemctl --user start pallet-daemon".dimmed()
        );
    }
}

pub fn history(engine: &Engine, limit: u32) -> Result<()> {
    let entries = engine.registry.recent_history(limit)?;
    if entries.is_empty() {
        println!("{}", "(no history yet)".dimmed());
        return Ok(());
    }
    for entry in entries {
        print_history_entry(&entry);
    }
    Ok(())
}

fn print_history_entry(entry: &HistoryEntry) {
    let mark = if entry.success {
        "✓".green()
    } else {
        "✗".red()
    };
    println!(
        "{} {} {:<8} {:<10} {}",
        entry
            .timestamp
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M"),
        mark,
        entry.action,
        entry.source,
        entry.package,
    );
}

pub fn pin(engine: &Engine, source: SourceId, id: &str) -> Result<()> {
    engine.registry.pin(source, id)?;
    output::print_success(&format!(
        "Pinned {id} ({source}); it will be skipped by bulk updates."
    ));
    Ok(())
}

pub fn unpin(engine: &Engine, source: SourceId, id: &str) -> Result<()> {
    engine.registry.unpin(source, id)?;
    output::print_success(&format!("Unpinned {id} ({source})."));
    Ok(())
}

pub fn setup(what: &SetupCommand, assume_yes: bool) -> Result<()> {
    match what {
        SetupCommand::Sudoers => setup_sudoers(assume_yes),
        SetupCommand::Daemon => setup_daemon(assume_yes),
    }
}

fn current_user() -> Result<String> {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .context("could not determine the current username (no $USER or $LOGNAME)")
}

fn setup_sudoers(assume_yes: bool) -> Result<()> {
    let user = current_user()?;
    let rule = pallet_core::privilege::render_sudoers_rule(&user)?;
    println!("The following rule will be installed at /etc/sudoers.d/pallet:\n");
    println!("{}", rule.dimmed());
    if !output::confirm("Install this sudoers rule?", assume_yes) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let path = pallet_core::privilege::install_sudoers_rule(&user)?;
    output::print_success(&format!("Installed {}", path.display()));
    Ok(())
}

fn setup_daemon(assume_yes: bool) -> Result<()> {
    let daemon_binary = locate_daemon_binary()?;
    let unit_dir = dirs_next_config_home().join("systemd/user");
    std::fs::create_dir_all(&unit_dir)?;
    let unit_path = unit_dir.join("pallet-daemon.service");
    let unit_contents = format!(
        "[Unit]\n\
         Description=Pallet background update monitor\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=5\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        daemon_binary.display()
    );
    println!("Writing {}", unit_path.display());
    if !output::confirm(
        "Install and enable the pallet-daemon user service?",
        assume_yes,
    ) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    std::fs::write(&unit_path, unit_contents)?;
    run_systemctl(&["--user", "daemon-reload"])?;
    run_systemctl(&["--user", "enable", "--now", "pallet-daemon.service"])?;
    output::print_success(
        "pallet-daemon is enabled and running (systemctl --user status pallet-daemon)",
    );
    Ok(())
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()?;
    if !status.success() {
        bail!("systemctl {} failed", args.join(" "));
    }
    Ok(())
}

fn locate_daemon_binary() -> Result<PathBuf> {
    if let Some(path) = pallet_core::process::which("pallet-daemon") {
        return Ok(path);
    }
    let own = std::env::current_exe().context("could not determine this binary's own path")?;
    if let Some(dir) = own.parent() {
        let candidate = dir.join("pallet-daemon");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!("could not find the pallet-daemon binary; install it alongside pallet or put it on PATH")
}

fn dirs_next_config_home() -> PathBuf {
    dirs_next::config_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn config(what: &ConfigCommand) -> Result<()> {
    match what {
        ConfigCommand::Show => {
            let config = Config::load()?;
            println!(
                "location: {}",
                pallet_core::config::Paths::config_file().display()
            );
            println!("{}", toml::to_string_pretty(&config)?);
        }
        ConfigCommand::Set { key, value } => {
            let mut config = Config::load()?;
            apply_config_set(&mut config, key, value)?;
            config.save()?;
            output::print_success(&format!("Set {key} = {value}"));
        }
    }
    Ok(())
}

fn apply_config_set(config: &mut Config, key: &str, value: &str) -> Result<()> {
    match key {
        "check-interval-minutes" => {
            config.check_interval_minutes = value.parse().context("expected a positive integer")?;
        }
        "update-policy" => {
            config.update_policy = match value {
                "notify-only" => UpdatePolicy::NotifyOnly,
                "security-only" => UpdatePolicy::SecurityOnly,
                "auto-all" => UpdatePolicy::AutoAll,
                other => bail!(
                    "unknown update-policy '{other}' (expected notify-only, security-only, or auto-all)"
                ),
            };
        }
        "notifications-enabled" => {
            config.notifications_enabled = value.parse().context("expected true or false")?;
        }
        "require-checksum-for-appimage" => {
            config.require_checksum_for_appimage =
                value.parse().context("expected true or false")?;
        }
        "require-checksum-for-archive" => {
            config.require_checksum_for_archive =
                value.parse().context("expected true or false")?;
        }
        other => bail!("unknown config key '{other}'"),
    }
    Ok(())
}

fn find_owning_source(engine: &Engine, id: &str) -> Result<SourceId> {
    let mut matches = Vec::new();
    for adapter in engine.active_sources() {
        if let Ok(pkgs) = adapter.list_installed()
            && pkgs.iter().any(|p| p.id == id)
        {
            matches.push(adapter.id());
        }
    }
    match matches.len() {
        0 => bail!(
            "'{id}' was not found in any installed source; pass --source to target a specific one"
        ),
        1 => Ok(matches[0]),
        _ => bail!(
            "'{id}' exists in more than one source ({}); pass --source to disambiguate",
            matches
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn report(outcome: &OperationOutcome) {
    if outcome.success {
        output::print_success(&outcome.summary);
    } else {
        output::print_failure(&outcome.summary);
    }
    if std::env::var("PALLET_VERBOSE").is_ok() && !outcome.log.is_empty() {
        println!("{}", outcome.log.dimmed());
    }
}

fn log_history(
    engine: &Engine,
    source: SourceId,
    action: &str,
    package: &str,
    outcome: &OperationOutcome,
) -> Result<()> {
    engine
        .registry
        .log_history(&pallet_core::registry::HistoryEntry {
            timestamp: chrono::Utc::now(),
            source: source.as_str().to_string(),
            action: action.to_string(),
            package: package.to_string(),
            success: outcome.success,
            detail: outcome.summary.clone(),
        })?;
    Ok(())
}
