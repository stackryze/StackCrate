use colored::Colorize;
use stackcrate_core::types::{Package, SourceId, UpdateInfo, UpdateKind, format_size_kb};
use std::io::Write;

pub fn confirm(prompt: &str, assume_yes: bool) -> bool {
    if assume_yes {
        return true;
    }
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        eprintln!(
            "{} refusing to proceed without --yes in a non-interactive session",
            "error:".red().bold()
        );
        return false;
    }
    eprint!("{prompt} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return false;
    }
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

const SOURCE_BADGE_WIDTH: usize = 8; // len("appimage") == len("deb-file"), the longest SourceId name

pub fn source_label(source: SourceId) -> colored::ColoredString {
    let padded = format!("{:<width$}", source.as_str(), width = SOURCE_BADGE_WIDTH);
    match source {
        SourceId::Apt | SourceId::DebFile => padded.blue(),
        SourceId::Archive | SourceId::AppImage => padded.purple(),
        SourceId::Flatpak => padded.cyan(),
        SourceId::Snap => padded.yellow(),
    }
}

pub fn print_packages(packages: &[Package]) {
    if packages.is_empty() {
        println!("{}", "(nothing found)".dimmed());
        return;
    }
    let name_width = packages
        .iter()
        .map(|p| p.name.len())
        .max()
        .unwrap_or(4)
        .clamp(4, 40);
    for pkg in packages {
        let version = if pkg.version.is_empty() {
            String::new()
        } else {
            format!(" {}", pkg.version.dimmed())
        };
        println!(
            "[{}] {:<width$}{}  {}",
            source_label(pkg.source.unwrap_or(SourceId::Apt)),
            pkg.name,
            version,
            pkg.description.dimmed(),
            width = name_width
        );
    }
}

pub fn print_updates(updates: &[UpdateInfo]) {
    if updates.is_empty() {
        println!("{}", "Everything is up to date.".green());
        return;
    }
    let security = updates
        .iter()
        .filter(|u| u.kind == UpdateKind::Security)
        .count();
    println!(
        "{} update(s) available ({} security)\n",
        updates.len().to_string().bold(),
        security.to_string().red().bold()
    );
    let name_width = updates
        .iter()
        .map(|u| u.name.len())
        .max()
        .unwrap_or(4)
        .clamp(4, 40);
    for update in updates {
        let marker = if update.kind == UpdateKind::Security {
            "[security]".red().bold()
        } else {
            "          ".normal()
        };
        println!(
            "[{}] {} {:<width$} {} -> {}",
            source_label(update.source),
            marker,
            update.name,
            update.current_version.dimmed(),
            update.new_version.green(),
            width = name_width
        );
    }
}

pub fn print_success(message: &str) {
    println!("{} {message}", "✓".green().bold());
}

pub fn print_failure(message: &str) {
    eprintln!("{} {message}", "✗".red().bold());
}

/// Pretty-prints any serializable value as JSON, for `--json` mode. Used
/// uniformly for every JSON-capable command instead of a bespoke function
/// per output shape.
pub fn print_json(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("{} failed to serialize as JSON: {e}", "error:".red().bold()),
    }
}

pub fn print_package_details(pkg: &Package) {
    println!("{}", pkg.name.bold());
    if pkg.name != pkg.id {
        println!("  id:          {}", pkg.id);
    }
    println!("  source:      {}", pkg.source.unwrap_or(SourceId::Apt));
    println!(
        "  installed:   {}",
        if pkg.installed {
            "yes".green().to_string()
        } else {
            "no".to_string()
        }
    );
    if !pkg.version.is_empty() {
        println!("  version:     {}", pkg.version);
    }
    if let Some(available) = &pkg.available_version {
        println!("  update:      {} -> {}", pkg.version, available.green());
    }
    if let Some(size_kb) = pkg.size_kb {
        println!("  size:        {}", format_size_kb(size_kb));
    }
    if let Some(maintainer) = &pkg.maintainer {
        println!("  maintainer:  {maintainer}");
    }
    if let Some(homepage) = &pkg.homepage {
        println!("  homepage:    {homepage}");
    }
    if let Some(license) = &pkg.license {
        println!("  license:     {license}");
    }
    if let Some(manual) = pkg.manually_installed {
        println!(
            "  requested:   {}",
            if manual {
                "explicitly"
            } else {
                "no (pulled in as a dependency)"
            }
        );
    }
    if !pkg.description.is_empty() {
        println!("\n{}", pkg.description);
    }
}

pub fn print_doctor(checks: &[stackcrate_core::DoctorCheck]) {
    let name_width = checks
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(4)
        .clamp(4, 50);
    for check in checks {
        let mark = if check.ok {
            "✓".green().bold()
        } else {
            "✗".red().bold()
        };
        println!(
            "{mark} {:<width$}  {}",
            check.name,
            check.detail.dimmed(),
            width = name_width
        );
    }
}

pub fn print_disk_usage(entries: &[stackcrate_core::types::DiskUsageEntry]) {
    let total_kb: u64 = entries.iter().map(|e| e.total_kb).sum();
    for entry in entries {
        if entry.package_count == 0 {
            continue;
        }
        let unknown = if entry.unknown_size_count > 0 {
            format!(" (+{} of unknown size)", entry.unknown_size_count)
        } else {
            String::new()
        };
        println!(
            "[{}] {} across {} package(s){unknown}",
            source_label(entry.source),
            format_size_kb(entry.total_kb).green(),
            entry.package_count,
        );
    }
    println!("\n{}: {}", "Total".bold(), format_size_kb(total_kb).bold());
}
