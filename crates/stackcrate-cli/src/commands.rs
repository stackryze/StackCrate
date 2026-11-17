use crate::cli::{ConfigCommand, ListCommand, SetupCommand};
use crate::output;
use anyhow::{Context, Result, anyhow, bail};
use colored::Colorize;
use stackcrate_core::Engine;
use stackcrate_core::config::{Config, UpdatePolicy};
use stackcrate_core::registry::HistoryEntry;
use stackcrate_core::sources::{
    self, appimage::AppImageInstallOptions, archive::ArchiveInstallOptions,
};
use stackcrate_core::types::{InstallTarget, OperationOutcome, SourceId};
use std::path::PathBuf;
use std::time::Duration;

const IPC_TIMEOUT: Duration = Duration::from_secs(10);

pub fn search(engine: &Engine, query: &str, source: Option<SourceId>, json: bool) -> Result<()> {
    match source {
        Some(id) => {
            let results = engine
                .search_one(id, query)
                .with_context(|| format!("searching {id}"))?;
            if json {
                output::print_json(&results);
            } else {
                output::print_packages(&results);
            }
        }
        None => {
            if json {
                let all: Vec<_> = engine
                    .search_all(query)
                    .into_iter()
                    .flat_map(|(_, result)| result.unwrap_or_default())
                    .collect();
                output::print_json(&all);
                return Ok(());
            }
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
                ..Default::default()
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
    let cache_dir = stackcrate_core::config::Paths::cache_dir().join("downloads");
    let downloaded = stackcrate_core::download::download(url, &cache_dir)
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
        stackcrate_core::verify::verify_checksum(&downloaded.path, expected)
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

    if source_id == SourceId::Apt {
        let extra = sources::apt::removal_preview(id).unwrap_or_default();
        if !extra.is_empty() {
            println!(
                "{} removing {id} will also remove {} other package(s):",
                "Note:".yellow().bold(),
                extra.len()
            );
            for name in &extra {
                println!("  {name}");
            }
        }
    }

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

pub fn autoremove(engine: &Engine, assume_yes: bool) -> Result<()> {
    let orphans = engine.list_orphans()?;
    if orphans.is_empty() {
        output::print_success("Nothing to remove; no orphaned packages found.");
        return Ok(());
    }
    output::print_packages(&orphans);
    let total_kb: u64 = orphans.iter().filter_map(|p| p.size_kb).sum();
    if total_kb > 0 {
        println!(
            "\nThis will free about {}.",
            stackcrate_core::types::format_size_kb(total_kb).green()
        );
    }
    if !output::confirm(
        &format!("Remove these {} package(s)?", orphans.len()),
        assume_yes,
    ) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let outcome = engine.autoremove()?;
    report(&outcome);
    log_history(engine, SourceId::Apt, "autoremove", "(orphans)", &outcome)?;
    if !outcome.success {
        bail!("autoremove failed");
    }
    Ok(())
}

pub fn clean() -> Result<()> {
    let dir = stackcrate_core::config::Paths::cache_dir().join("downloads");
    if !dir.exists() {
        output::print_success("Nothing to clean; the download cache is already empty.");
        return Ok(());
    }
    let freed_kb = dir_size_kb(&dir);
    std::fs::remove_dir_all(&dir)?;
    std::fs::create_dir_all(&dir)?;
    output::print_success(&format!(
        "Removed cached downloads ({}).",
        stackcrate_core::types::format_size_kb(freed_kb)
    ));
    Ok(())
}

fn dir_size_kb(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let bytes: u64 = entries
        .flatten()
        .map(|entry| entry.metadata().map(|m| m.len()).unwrap_or(0))
        .sum();
    bytes / 1024
}

pub fn rollback(engine: &Engine, source: SourceId, id: &str, assume_yes: bool) -> Result<()> {
    let snapshots = engine.list_snapshots(source, id)?;
    let Some(latest) = snapshots.first() else {
        bail!("no snapshot available to roll back to for '{id}' ({source})");
    };
    println!(
        "Will restore {} {} from the snapshot taken {}.",
        latest.manual_install.name,
        latest.manual_install.version,
        latest
            .created_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M")
    );
    if !output::confirm("Roll back to this snapshot?", assume_yes) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let outcome = engine.rollback(source, id)?;
    report(&outcome);
    log_history(engine, source, "rollback", id, &outcome)?;
    if !outcome.success {
        bail!("rollback failed");
    }
    Ok(())
}

pub fn doctor(engine: &Engine, json: bool) -> Result<()> {
    let checks = engine.doctor();
    if json {
        output::print_json(&checks);
    } else {
        output::print_doctor(&checks);
    }
    if checks.iter().any(|c| !c.ok) {
        bail!("one or more checks failed");
    }
    Ok(())
}

pub fn explain(engine: &Engine, source: SourceId, id: &str, json: bool) -> Result<()> {
    let info = engine.explain(source, id)?;
    if json {
        output::print_json(&info);
        return Ok(());
    }
    match info.manually_installed {
        Some(true) => {
            println!("{id} was explicitly installed (not just pulled in as a dependency).")
        }
        Some(false) => {
            println!("{id} was installed automatically, as a dependency of something else.")
        }
        None => println!(
            "{id}: installation reason unknown ({source} may not track this, or it isn't installed)."
        ),
    }
    if info.required_by.is_empty() {
        println!("Nothing currently installed depends on it.");
    } else {
        println!("Required by:");
        for dep in &info.required_by {
            println!("  {dep}");
        }
    }
    Ok(())
}

pub fn changelog(name: &str) -> Result<()> {
    let text = stackcrate_core::sources::apt::changelog(name)?;
    print!("{text}");
    Ok(())
}

pub fn show(engine: &Engine, source: SourceId, id: &str, json: bool) -> Result<()> {
    let pkg = engine.package_details(source, id)?;
    if json {
        output::print_json(&pkg);
    } else {
        output::print_package_details(&pkg);
    }
    Ok(())
}

pub fn list(engine: &Engine, what: &ListCommand, json: bool) -> Result<()> {
    match what {
        ListCommand::Installed { source } => {
            let sources_to_query: Vec<Box<dyn sources::PackageSource>> = match source {
                Some(id) => vec![sources::source_by_id(*id)],
                None => engine.active_sources(),
            };
            if json {
                let mut all = Vec::new();
                for adapter in sources_to_query {
                    all.extend(adapter.list_installed().unwrap_or_default());
                }
                output::print_json(&all);
                return Ok(());
            }
            for adapter in sources_to_query {
                match adapter.list_installed() {
                    Ok(pkgs) => output::print_packages(&pkgs),
                    Err(e) => output::print_failure(&format!("{}: {e}", adapter.id())),
                }
            }
        }
        ListCommand::Updates => {
            let updates = engine.registry.cached_updates()?;
            if json {
                output::print_json(&updates);
            } else {
                output::print_updates(&updates);
            }
        }
        ListCommand::Sources => {
            if json {
                #[derive(serde::Serialize)]
                struct SourceStatus {
                    source: SourceId,
                    available: bool,
                    enabled: bool,
                }
                let statuses: Vec<_> = SourceId::all()
                    .iter()
                    .map(|source| SourceStatus {
                        source: *source,
                        available: sources::source_by_id(*source).is_available(),
                        enabled: engine.config.is_source_enabled(*source),
                    })
                    .collect();
                output::print_json(&statuses);
                return Ok(());
            }
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
        ListCommand::DiskUsage => {
            let entries = engine.disk_usage_summary();
            if json {
                output::print_json(&entries);
            } else {
                output::print_disk_usage(&entries);
            }
        }
    }
    Ok(())
}

pub fn status(engine: &Engine, json: bool) -> Result<()> {
    match stackcrate_core::ipc::request_blocking(
        &stackcrate_core::ipc::Request::Status,
        IPC_TIMEOUT,
    ) {
        Ok(stackcrate_core::ipc::Response::Status {
            last_checked_at,
            update_count,
            security_count,
        }) => {
            print_status(last_checked_at, update_count, security_count, true, json);
        }
        _ => {
            let updates = engine.registry.cached_updates()?;
            let security = updates
                .iter()
                .filter(|u| u.kind == stackcrate_core::types::UpdateKind::Security)
                .count();
            print_status(
                engine.registry.last_checked_at()?,
                updates.len(),
                security,
                false,
                json,
            );
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct StatusJson {
    daemon_running: bool,
    last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    update_count: usize,
    security_count: usize,
}

fn print_status(
    last_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    update_count: usize,
    security_count: usize,
    daemon_running: bool,
    json: bool,
) {
    if json {
        output::print_json(&StatusJson {
            daemon_running,
            last_checked_at,
            update_count,
            security_count,
        });
        return;
    }
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
            "Start the background monitor with: systemctl --user start stackcrate-daemon".dimmed()
        );
    }
}

pub fn history(engine: &Engine, limit: u32, json: bool) -> Result<()> {
    let entries = engine.registry.recent_history(limit)?;
    if json {
        output::print_json(&entries);
        return Ok(());
    }
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

pub fn pin(engine: &Engine, source: SourceId, id: &str, version: Option<String>) -> Result<()> {
    engine.registry.pin(source, id, version.as_deref())?;
    if source == SourceId::Apt {
        let outcome = stackcrate_core::sources::apt::hold(id)?;
        if !outcome.success {
            output::print_failure(&format!(
                "recorded the pin, but `apt-mark hold {id}` failed: {}",
                outcome.summary
            ));
        }
    }
    match version {
        Some(v) => output::print_success(&format!(
            "Pinned {id} ({source}) to version {v}; it will be skipped by bulk updates."
        )),
        None => output::print_success(&format!(
            "Pinned {id} ({source}); it will be skipped by bulk updates."
        )),
    }
    Ok(())
}

pub fn unpin(engine: &Engine, source: SourceId, id: &str) -> Result<()> {
    engine.registry.unpin(source, id)?;
    if source == SourceId::Apt {
        let outcome = stackcrate_core::sources::apt::unhold(id)?;
        if !outcome.success {
            output::print_failure(&format!(
                "removed the pin, but `apt-mark unhold {id}` failed: {}",
                outcome.summary
            ));
        }
    }
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
    let rule = stackcrate_core::privilege::render_sudoers_rule(&user)?;
    println!("The following rule will be installed at /etc/sudoers.d/stackcrate:\n");
    println!("{}", rule.dimmed());
    if !output::confirm("Install this sudoers rule?", assume_yes) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    let path = stackcrate_core::privilege::install_sudoers_rule(&user)?;
    output::print_success(&format!("Installed {}", path.display()));
    Ok(())
}

fn setup_daemon(assume_yes: bool) -> Result<()> {
    let daemon_binary = stackcrate_core::setup::locate_daemon_binary()?;
    let unit_path = stackcrate_core::setup::daemon_unit_path();
    println!("Writing {}", unit_path.display());
    println!("  ExecStart={}", daemon_binary.display());
    if !output::confirm(
        "Install and enable the stackcrate-daemon user service?",
        assume_yes,
    ) {
        println!("{}", "Aborted.".dimmed());
        return Ok(());
    }
    stackcrate_core::setup::install_and_start_daemon_unit()?;
    output::print_success(
        "stackcrate-daemon is enabled and running (systemctl --user status stackcrate-daemon)",
    );
    Ok(())
}

pub fn config(what: &ConfigCommand) -> Result<()> {
    match what {
        ConfigCommand::Show => {
            let config = Config::load()?;
            println!(
                "location: {}",
                stackcrate_core::config::Paths::config_file().display()
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
        "require-ac-power-for-auto-update" => {
            config.require_ac_power_for_auto_update =
                value.parse().context("expected true or false")?;
        }
        other => bail!("unknown config key '{other}'"),
    }
    Ok(())
}

pub fn completions(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(shell, &mut cmd, "crate", &mut std::io::stdout());
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
    if std::env::var("STACKCRATE_VERBOSE").is_ok() && !outcome.log.is_empty() {
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
        .log_history(&stackcrate_core::registry::HistoryEntry {
            timestamp: chrono::Utc::now(),
            source: source.as_str().to_string(),
            action: action.to_string(),
            package: package.to_string(),
            success: outcome.success,
            detail: outcome.summary.clone(),
        })?;
    Ok(())
}
