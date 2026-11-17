//! One-time local setup shared between the CLI and GUI: writing the
//! daemon's systemd `--user` unit and enabling it. The sudoers rule itself
//! lives in `privilege.rs`; this is just the systemd half, kept here so
//! both frontends install the exact same unit instead of two copies that
//! could drift apart.

use crate::error::{Error, Result};
use std::path::PathBuf;

pub fn daemon_unit_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("systemd/user/stackcrate-daemon.service")
}

pub fn locate_daemon_binary() -> Result<PathBuf> {
    if let Some(path) = crate::process::which("stackcrate-daemon") {
        return Ok(path);
    }
    let own = std::env::current_exe().map_err(Error::Io)?;
    if let Some(dir) = own.parent() {
        let candidate = dir.join("stackcrate-daemon");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(Error::other(
        "could not find the stackcrate-daemon binary; install it alongside crate/stackcrate-gui, or put it on PATH",
    ))
}

pub fn render_daemon_unit(daemon_binary: &std::path::Path) -> String {
    format!(
        "[Unit]\n\
         Description=StackCrate background update monitor\n\
         After=network-online.target\n\
         Wants=network-online.target\n\n\
         [Service]\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=5\n\n\
         [Install]\n\
         WantedBy=default.target\n",
        daemon_binary.display()
    )
}

/// Writes the systemd `--user` unit and runs `systemctl --user enable --now`.
/// Needs `systemctl` on PATH; no privilege escalation (it's a user unit).
pub fn install_and_start_daemon_unit() -> Result<PathBuf> {
    let daemon_binary = locate_daemon_binary()?;
    let unit_path = daemon_unit_path();
    if let Some(parent) = unit_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&unit_path, render_daemon_unit(&daemon_binary))?;
    run_systemctl(&["--user", "daemon-reload"])?;
    run_systemctl(&["--user", "enable", "--now", "stackcrate-daemon.service"])?;
    Ok(unit_path)
}

fn run_systemctl(args: &[&str]) -> Result<()> {
    crate::process::require_binary("systemctl")?;
    let status = std::process::Command::new("systemctl")
        .args(args)
        .status()
        .map_err(Error::Io)?;
    if !status.success() {
        return Err(Error::other(format!("systemctl {} failed", args.join(" "))));
    }
    Ok(())
}
