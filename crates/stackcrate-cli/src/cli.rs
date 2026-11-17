use clap::{Parser, Subcommand};
use stackcrate_core::types::SourceId;

#[derive(Parser, Debug)]
#[command(
    name = "crate",
    version,
    about = "A Linux package manager that unifies apt, local .deb files, archives, AppImages, Flatpak and Snap.",
    long_about = None,
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// Assume "yes" to any confirmation prompt.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,

    /// Print extra diagnostic logging.
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Search every available source for packages matching a query.
    Search {
        query: String,
        /// Restrict the search to one source (apt, flatpak, snap).
        #[arg(long)]
        source: Option<SourceId>,
    },

    /// Install something: a bare package name, a local file (.deb, archive,
    /// .AppImage), or an http(s) URL to one of those.
    Install {
        /// Package name, file path, or URL.
        target: String,
        /// Force which adapter handles this install instead of auto-detecting.
        #[arg(long)]
        source: Option<SourceId>,
        /// Display name to use for a manual archive/AppImage install.
        #[arg(long)]
        name: Option<String>,
        /// Path to the main executable inside an extracted archive, relative
        /// to the extraction directory (archive installs only).
        #[arg(long)]
        exec: Option<String>,
        /// Icon file to install alongside the launcher.
        #[arg(long)]
        icon: Option<std::path::PathBuf>,
        /// Don't create a `.desktop` launcher / PATH symlink.
        #[arg(long)]
        no_launcher: bool,
        /// Expected sha256 or sha512 checksum; required for URL installs
        /// when the config requires one (`require-checksum-for-*`).
        #[arg(long)]
        checksum: Option<String>,
    },

    /// Remove an installed package.
    Remove {
        id: String,
        #[arg(long)]
        source: Option<SourceId>,
    },

    /// Install available updates.
    Update {
        /// Package id to update; omit to update everything.
        id: Option<String>,
        #[arg(long)]
        source: Option<SourceId>,
        /// Refresh apt's repository metadata first (needs root; see `crate setup sudoers`).
        #[arg(long)]
        refresh: bool,
    },

    /// List things: installed packages, pending updates, or active sources.
    List {
        #[command(subcommand)]
        what: ListCommand,
    },

    /// Show the background monitor's last check time and pending-update counts.
    Status,

    /// Show recent install/remove/update history.
    History {
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Hold a package back from automatic/bulk updates.
    Pin { source: SourceId, id: String },

    /// Remove a hold placed with `pin`.
    Unpin { source: SourceId, id: String },

    /// One-time local setup: sudoers rule, systemd user service, config.
    Setup {
        #[command(subcommand)]
        what: SetupCommand,
    },

    /// View or change configuration.
    Config {
        #[command(subcommand)]
        what: ConfigCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum ListCommand {
    /// Everything currently installed, across all active sources.
    Installed {
        #[arg(long)]
        source: Option<SourceId>,
    },
    /// Pending updates (from the local cache; use `crate update --refresh` first for apt).
    Updates,
    /// Which adapters are available on this machine.
    Sources,
}

#[derive(Subcommand, Debug)]
pub enum SetupCommand {
    /// Install a narrow /etc/sudoers.d/stackcrate rule so background checks and
    /// updates don't repeatedly prompt for a password. Review before
    /// accepting; this is entirely optional.
    Sudoers,
    /// Install and enable the `stackcrate-daemon` systemd --user service.
    Daemon,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Print the active configuration (and where it's stored).
    Show,
    /// Set `check-interval-minutes`, `update-policy`, or `notifications-enabled`.
    Set { key: String, value: String },
}
