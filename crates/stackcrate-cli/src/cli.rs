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

    /// Print machine-readable JSON instead of formatted text, for scripting.
    /// Supported by the read-only commands (search, list, status, history,
    /// show); ignored elsewhere.
    #[arg(long, global = true)]
    pub json: bool,
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

    /// Remove apt packages that were pulled in as dependencies and are no
    /// longer required by anything still installed.
    Autoremove,

    /// Clear StackCrate's own download cache (`~/.cache/stackcrate/downloads`,
    /// used when installing from a URL). Nothing installed is affected.
    Clean,

    /// Restore the most recent snapshot of a manually-installed archive or
    /// AppImage, taken automatically the last time it was overwritten by a
    /// newer install.
    Rollback {
        /// Must be `archive` or `appimage`; other sources manage their own
        /// versions and don't need this.
        source: SourceId,
        id: String,
    },

    /// Check that StackCrate's pieces are healthy: which sources are
    /// available, whether the background daemon is reachable, and whether
    /// unattended privilege escalation is configured.
    Doctor,

    /// Show why a package is installed and what currently depends on it
    /// (apt only; other sources don't track this).
    Explain { source: SourceId, id: String },

    /// Show a package's upstream/Debian changelog (apt only; needs network
    /// access unless a local copy is already on disk).
    Changelog { name: String },

    /// Show full details for one package: size, maintainer, homepage,
    /// license, and whether it's installed, beyond what `search`/`list`
    /// carry.
    Show { source: SourceId, id: String },

    /// List things: installed packages, pending updates, active sources, or
    /// disk space used by each source's installed packages.
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
    Pin {
        source: SourceId,
        id: String,
        /// Pin to this exact version (recorded for display; apt additionally
        /// enforces the hold itself via `apt-mark hold`, independent of the
        /// version string).
        #[arg(long)]
        version: Option<String>,
    },

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

    /// Print a shell completion script to stdout, e.g.
    /// `crate completions bash >> ~/.bashrc` (packaged installs already ship
    /// these pre-generated; this is for anyone building/installing another way).
    Completions { shell: clap_complete::Shell },
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
    /// Disk space used by each source's installed packages.
    DiskUsage,
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
