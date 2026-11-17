use colored::Colorize;
use pallet_core::types::{Package, SourceId, UpdateInfo, UpdateKind};
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
