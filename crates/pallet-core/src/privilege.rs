//! Privilege escalation for operations that need root (installing/removing
//! system packages, writing to /etc/sudoers.d, etc).
//!
//! Strategy:
//! 1. Already root -> run directly.
//! 2. A graphical session is present (DISPLAY/WAYLAND_DISPLAY) and `pkexec` is
//!    available -> use pkexec, so polkit shows a native auth dialog. This is
//!    the preferred path for the GUI and for the daemon.
//! 3. Otherwise, if stdin is a real terminal and `sudo` is available -> use
//!    sudo, which prompts inline. This matters for CLI/SSH usage where pkexec
//!    has no agent to talk to and would just fail with "cannot open display".
//! 4. Otherwise, if `SUDO_ASKPASS` is set and sudo supports `-A` -> use that
//!    (covers a background daemon that still needs an askpass helper).
//! 5. Nothing available -> `Error::NoPrivilegeHelper`.

use crate::error::{Error, Result};
use crate::process::{self, CommandOutput};
use std::io::IsTerminal;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Direct,
    Pkexec,
    Sudo,
    SudoAskpass,
}

pub fn am_root() -> bool {
    nix::unistd::Uid::effective().is_root()
}

fn has_display() -> bool {
    std::env::var_os("DISPLAY").is_some() || std::env::var_os("WAYLAND_DISPLAY").is_some()
}

fn sudo_supports_askpass() -> bool {
    let Some(_) = process::which("sudo") else {
        return false;
    };
    match process::run("sudo", &["-h"], Duration::from_secs(5)) {
        Ok(out) => out.combined().contains("-A") || out.combined().contains("--askpass"),
        Err(_) => false,
    }
}

pub fn choose_method() -> Method {
    if am_root() {
        return Method::Direct;
    }
    if has_display() && process::binary_available("pkexec") {
        return Method::Pkexec;
    }
    if std::io::stdin().is_terminal() && process::binary_available("sudo") {
        return Method::Sudo;
    }
    if std::env::var_os("SUDO_ASKPASS").is_some() && sudo_supports_askpass() {
        return Method::SudoAskpass;
    }
    if process::binary_available("pkexec") {
        return Method::Pkexec;
    }
    if process::binary_available("sudo") {
        return Method::Sudo;
    }
    Method::Direct // caller will get BinaryMissing/CommandFailed naturally
}

/// Build the actual argv to execute for a command that needs root.
pub fn wrap(program: &str, args: &[String]) -> Result<(String, Vec<String>)> {
    if !process::binary_available(program) {
        return Err(Error::BinaryMissing {
            binary: program.to_string(),
        });
    }
    match choose_method() {
        Method::Direct => Ok((program.to_string(), args.to_vec())),
        Method::Pkexec => {
            let mut full = vec![program.to_string()];
            full.extend(args.iter().cloned());
            Ok(("pkexec".to_string(), full))
        }
        Method::Sudo => {
            let mut full = vec![program.to_string()];
            full.extend(args.iter().cloned());
            Ok(("sudo".to_string(), full))
        }
        Method::SudoAskpass => {
            let mut full = vec!["-A".to_string(), program.to_string()];
            full.extend(args.iter().cloned());
            Ok(("sudo".to_string(), full))
        }
    }
}

/// Run a privileged command and wait for it to finish.
pub fn run_privileged(program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput> {
    if !am_root() && !process::binary_available("pkexec") && !process::binary_available("sudo") {
        return Err(Error::NoPrivilegeHelper);
    }
    let (wrapped_program, wrapped_args) = wrap(program, args)?;
    process::run_owned(&wrapped_program, &wrapped_args, timeout)
}

/// Probe whether `program` can run under `sudo` right now with no password
/// prompt at all (`sudo -n` fails fast instead of prompting). Used by the
/// background daemon to decide whether an unattended metadata refresh is
/// possible, so a timer-driven check never pops a surprise auth dialog; it
/// only does so after the user has explicitly installed the narrow sudoers
/// rule via `pallet setup sudoers`, or is already root.
pub fn passwordless_available(program: &str) -> bool {
    if am_root() {
        return true;
    }
    let Some(path) = process::which(program) else {
        return false;
    };
    if !process::binary_available("sudo") {
        return false;
    }
    matches!(
        process::run(
            "sudo",
            &["-n", &path.to_string_lossy(), "--version"],
            Duration::from_secs(5),
        ),
        Ok(out) if out.success()
    )
}

/// Full paths of system package-manager binaries present on this machine,
/// used both for the sudoers allowlist and for diagnostics.
pub fn known_package_manager_binaries() -> Vec<std::path::PathBuf> {
    [
        "apt-get",
        "dpkg",
        "apt-cache",
        "dnf5",
        "dnf",
        "zypper",
        "pacman",
        "apk",
        "xbps-install",
        "xbps-remove",
        "snap",
        "flatpak",
    ]
    .iter()
    .filter_map(|b| process::which(b))
    .collect()
}

/// Render a narrow sudoers.d rule that allows the current user to run only
/// the detected system package-manager binaries without a password. This is
/// never installed automatically; callers (CLI `pallet setup sudoers`, or the
/// GUI's first-run flow) must ask for explicit confirmation first.
pub fn render_sudoers_rule(user: &str) -> Result<String> {
    let bins = known_package_manager_binaries();
    if bins.is_empty() {
        return Err(Error::other(
            "no known system package-manager binaries were found",
        ));
    }
    let joined = bins
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Ok(format!(
        "# Installed by Pallet. Allows {user} to run specific package managers\n\
         # without a repeated password prompt. Review before trusting.\n\
         Cmnd_Alias PALLET_PACKAGE_MANAGERS = {joined}\n\
         Defaults:{user} env_keep += \"SUDO_ASKPASS DISPLAY WAYLAND_DISPLAY XAUTHORITY DBUS_SESSION_BUS_ADDRESS\"\n\
         {user} ALL=(root) NOPASSWD: PALLET_PACKAGE_MANAGERS\n"
    ))
}

/// Validate and install a sudoers rule at `/etc/sudoers.d/pallet`. Requires
/// `visudo` to be present so the rule is syntax-checked before it is ever
/// installed. Returns the path written.
pub fn install_sudoers_rule(user: &str) -> Result<std::path::PathBuf> {
    process::require_binary("visudo")?;
    let content = render_sudoers_rule(user)?;
    let tmp = tempfile::Builder::new()
        .prefix("pallet-sudoers-")
        .tempfile()?;
    std::fs::write(tmp.path(), &content)?;

    let check = process::run(
        "visudo",
        &["-cf", tmp.path().to_string_lossy().as_ref()],
        Duration::from_secs(10),
    )?;
    if !check.success() {
        return Err(Error::other(format!(
            "generated sudoers rule failed validation:\n{}",
            check.combined()
        )));
    }

    let target = "/etc/sudoers.d/pallet";
    let args = vec![
        "-m".to_string(),
        "0440".to_string(),
        tmp.path().to_string_lossy().to_string(),
        target.to_string(),
    ];
    let out = run_privileged("install", &args, Duration::from_secs(10))?;
    if !out.success() {
        return Err(Error::CommandFailed {
            command: format!("install -m 0440 {} {target}", tmp.path().display()),
            code: out.code,
            output: out.combined(),
        });
    }
    Ok(std::path::PathBuf::from(target))
}
