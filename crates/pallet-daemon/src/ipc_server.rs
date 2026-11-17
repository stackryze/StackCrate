//! Async server side of the CLI/GUI <-> daemon Unix-socket protocol defined
//! in `pallet_core::ipc`.

use pallet_core::Engine;
use pallet_core::ipc::{Request, Response};
use pallet_core::types::{OperationOutcome, SourceId};
use std::os::unix::net::UnixStream as StdUnixStream;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

pub async fn serve(mut shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()> {
    let path = pallet_core::ipc::socket_path();
    prepare_socket_path(&path).await?;

    let listener = UnixListener::bind(&path)?;
    info!("listening on {}", path.display());

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _addr)) => {
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(stream).await {
                                warn!("ipc connection error: {e}");
                            }
                        });
                    }
                    Err(e) => warn!("accept failed: {e}"),
                }
            }
            _ = shutdown.changed() => {
                info!("shutting down ipc server");
                let _ = std::fs::remove_file(&path);
                return Ok(());
            }
        }
    }
}

/// Remove a stale socket file left behind by a previous crash, but refuse to
/// start if another daemon instance is genuinely still listening.
async fn prepare_socket_path(path: &std::path::Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if !path.exists() {
        return Ok(());
    }
    let probe_path = path.to_path_buf();
    let already_running =
        tokio::task::spawn_blocking(move || StdUnixStream::connect(&probe_path).is_ok()).await?;
    if already_running {
        anyhow::bail!(
            "another pallet-daemon is already listening on {}",
            path.display()
        );
    }
    tokio::fs::remove_file(path).await?;
    Ok(())
}

async fn handle_connection(stream: UnixStream) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle_request(request).await,
            Err(e) => Response::Error {
                message: format!("invalid request: {e}"),
            },
        };
        let mut out = serde_json::to_string(&response)?;
        out.push('\n');
        writer.write_all(out.as_bytes()).await?;
    }
    Ok(())
}

async fn handle_request(request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong,
        Request::Status => tokio::task::spawn_blocking(status_blocking)
            .await
            .unwrap_or_else(error_response),
        Request::CheckNow => {
            crate::scheduler::run_check_cycle().await;
            tokio::task::spawn_blocking(cached_updates_blocking)
                .await
                .unwrap_or_else(error_response)
        }
        Request::CachedUpdates => tokio::task::spawn_blocking(cached_updates_blocking)
            .await
            .unwrap_or_else(error_response),
        Request::Apply { items } => tokio::task::spawn_blocking(move || apply_blocking(items))
            .await
            .unwrap_or_else(error_response),
    }
}

fn error_response(join_error: tokio::task::JoinError) -> Response {
    Response::Error {
        message: format!("internal error: {join_error}"),
    }
}

fn status_blocking() -> Response {
    match Engine::open() {
        Ok(engine) => {
            let updates = engine.registry.cached_updates().unwrap_or_default();
            let security = updates
                .iter()
                .filter(|u| u.kind == pallet_core::types::UpdateKind::Security)
                .count();
            Response::Status {
                last_checked_at: engine.registry.last_checked_at().ok().flatten(),
                update_count: updates.len(),
                security_count: security,
            }
        }
        Err(e) => Response::Error {
            message: e.to_string(),
        },
    }
}

fn cached_updates_blocking() -> Response {
    match Engine::open() {
        Ok(engine) => match engine.registry.cached_updates() {
            Ok(items) => Response::Updates { items },
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
        Err(e) => Response::Error {
            message: e.to_string(),
        },
    }
}

fn apply_blocking(items: Vec<(SourceId, String)>) -> Response {
    let engine = match Engine::open() {
        Ok(e) => e,
        Err(e) => {
            return Response::Error {
                message: e.to_string(),
            };
        }
    };
    let mut results = Vec::new();

    if items.is_empty() {
        let cached = engine.registry.cached_updates().unwrap_or_default();
        let sources_with_updates: std::collections::BTreeSet<SourceId> =
            cached.iter().map(|u| u.source).collect();
        for source in sources_with_updates {
            let outcome = run_update(&engine, source, None);
            results.push((source, "(all)".to_string(), outcome));
        }
    } else {
        for (source, id) in items {
            let outcome = run_update(&engine, source, Some(&id));
            results.push((source, id, outcome));
        }
    }
    Response::Applied { results }
}

fn run_update(engine: &Engine, source: SourceId, id: Option<&str>) -> OperationOutcome {
    let outcome = match pallet_core::sources::source_by_id(source).update(id) {
        Ok(outcome) => outcome,
        Err(e) => OperationOutcome::failed(format!("{source} update failed"), e.to_string()),
    };
    let _ = engine
        .registry
        .log_history(&pallet_core::registry::HistoryEntry {
            timestamp: chrono::Utc::now(),
            source: source.as_str().to_string(),
            action: "update".to_string(),
            package: id.unwrap_or("(all)").to_string(),
            success: outcome.success,
            detail: outcome.summary.clone(),
        });
    if !outcome.success {
        error!(
            "update of {} ({source}) failed: {}",
            id.unwrap_or("(all)"),
            outcome.log
        );
    }
    outcome
}
