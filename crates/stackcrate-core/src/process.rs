//! Helpers for running external commands and capturing their output. Every
//! package-manager invocation in this codebase goes through here so logging,
//! timeouts, and error shapes stay consistent.

use crate::error::{Error, Result};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Output of a finished command.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn combined(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else {
            format!("{}{}", self.stdout, self.stderr)
        }
    }

    pub fn success(&self) -> bool {
        self.code == 0
    }
}

fn render(program: &str, args: &[String]) -> String {
    let mut out = String::from(program);
    for a in args {
        out.push(' ');
        out.push_str(&shell_quote(a));
    }
    out
}

/// Quote a single argument the way POSIX shells expect, used only for logging
/// and for the handful of places we must build a literal shell string (e.g.
/// pkexec fallback chains). Never used to build a string that gets re-parsed
/// by a shell for real execution; real execution always uses `Command` with an
/// argv array.
pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@%_+=:,./-".contains(&b))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Run a command, waiting up to `timeout` for it to finish, and capture stdout/stderr.
/// Never returns an `Err` for a non-zero exit code; callers decide whether that
/// matters. `Err` only indicates the command could not be spawned or timed out.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<CommandOutput> {
    run_owned(
        program,
        &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        timeout,
    )
}

pub fn run_owned(program: &str, args: &[String], timeout: Duration) -> Result<CommandOutput> {
    run_in(program, args, None, timeout)
}

pub fn run_in(
    program: &str,
    args: &[String],
    cwd: Option<&std::path::Path>,
    timeout: Duration,
) -> Result<CommandOutput> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::BinaryMissing {
                    binary: program.to_string(),
                }
            } else {
                Error::Io(e)
            }
        })?;

    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(p) = stdout_pipe.as_mut() {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buf = String::new();
        if let Some(p) = stderr_pipe.as_mut() {
            let _ = p.read_to_string(&mut buf);
        }
        buf
    });

    let start = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().map_err(Error::Io)? {
            break status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::CommandTimeout {
                command: render(program, args),
                seconds: timeout.as_secs(),
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    Ok(CommandOutput {
        code: status.code().unwrap_or(-1),
        stdout,
        stderr,
    })
}

/// Like `run`, but turns a non-zero exit code into `Error::CommandFailed`.
pub fn run_checked(program: &str, args: &[&str], timeout: Duration) -> Result<CommandOutput> {
    let out = run(program, args, timeout)?;
    if out.success() {
        Ok(out)
    } else {
        Err(Error::CommandFailed {
            command: render(
                program,
                &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            ),
            code: out.code,
            output: out.combined(),
        })
    }
}

/// Resolve a binary on PATH (plus a few well-known extra install locations).
pub fn which(binary: &str) -> Option<std::path::PathBuf> {
    which::which(binary).ok()
}

pub fn binary_available(binary: &str) -> bool {
    which(binary).is_some()
}

pub fn require_binary(binary: &str) -> Result<std::path::PathBuf> {
    which(binary).ok_or_else(|| Error::BinaryMissing {
        binary: binary.to_string(),
    })
}
