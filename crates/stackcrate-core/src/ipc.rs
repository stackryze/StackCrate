//! Control-plane IPC between the CLI/GUI and the background daemon: a Unix
//! domain socket carrying one newline-delimited JSON message per request and
//! per response. Deliberately simple (no D-Bus) so it has no dependency
//! surface beyond serde, is trivial to debug by hand (`socat - UNIX-CONNECT`),
//! and needs nothing beyond what stackcrate-core already pulls in.

use crate::config::Paths;
use crate::error::{Error, Result};
use crate::types::{OperationOutcome, SourceId, UpdateInfo};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    /// Lightweight liveness check.
    Ping,
    /// Current cached state: when it was last checked and how many updates
    /// are pending, without doing any work.
    Status,
    /// Ask the daemon to check all sources right now (unprivileged soft
    /// check; see `Engine::check_updates_all`) and return the fresh list.
    CheckNow,
    /// Return whatever is currently cached, without checking again.
    CachedUpdates,
    /// Apply updates. An empty `items` list means "everything cached".
    Apply { items: Vec<(SourceId, String)> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Pong,
    Status {
        last_checked_at: Option<DateTime<Utc>>,
        update_count: usize,
        security_count: usize,
    },
    Updates {
        items: Vec<UpdateInfo>,
    },
    Applied {
        results: Vec<(SourceId, String, OperationOutcome)>,
    },
    Error {
        message: String,
    },
}

pub fn socket_path() -> std::path::PathBuf {
    Paths::socket_path()
}

/// Send one request and wait for one response. Used by the CLI/GUI; the
/// daemon's own accept loop lives in the `stackcrate-daemon` binary since it
/// needs the async runtime and engine state.
pub fn request_blocking(request: &Request, timeout: Duration) -> Result<Response> {
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        Error::other(format!(
            "could not reach the StackCrate daemon at {} ({e}); is it running? (systemctl --user status stackcrate-daemon)",
            path.display()
        ))
    })?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    let n = reader.read_line(&mut response_line)?;
    if n == 0 {
        return Err(Error::other(
            "daemon closed the connection without a response",
        ));
    }
    Ok(serde_json::from_str(response_line.trim_end())?)
}
