mod ipc_server;
mod notify;
mod scheduler;
mod watch;

use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pallet_daemon=info".into()),
        )
        .with_target(false)
        .init();

    pallet_core::config::ensure_dirs()?;
    info!("pallet-daemon starting");

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let ipc_task = tokio::spawn(ipc_server::serve(shutdown_rx.clone()));
    let scheduler_task = tokio::spawn(scheduler_loop(shutdown_rx.clone()));

    wait_for_shutdown_signal().await;
    info!("shutdown signal received");
    let _ = shutdown_tx.send(true);

    let _ = scheduler_task.await;
    match ipc_task.await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => error!("ipc server exited with error: {e}"),
        Err(e) => error!("ipc server task panicked: {e}"),
    }
    info!("pallet-daemon stopped");
    Ok(())
}

async fn wait_for_shutdown_signal() {
    let mut sigterm = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            error!("could not install SIGTERM handler: {e}");
            std::future::pending::<()>().await;
            unreachable!()
        }
    };
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::select! {
        _ = sigterm.recv() => {}
        _ = ctrl_c => {}
    }
}

async fn scheduler_loop(mut shutdown: tokio::sync::watch::Receiver<bool>) {
    let mut fs_events = watch::spawn();
    // However often the filesystem genuinely changes (and on at least one
    // sandbox this was observed to be noisy well beyond real package
    // activity), fs-triggered checks are capped at this rate so an unusual
    // environment can't turn "react quickly to real changes" into a
    // CPU/subprocess busy-loop. The regular timer below is unaffected.
    const MIN_FS_TRIGGER_GAP: std::time::Duration = std::time::Duration::from_secs(60);
    let mut last_fs_triggered_check = std::time::Instant::now() - MIN_FS_TRIGGER_GAP;

    // Run one check right away so `pallet status` has something to show
    // immediately after the service starts, instead of waiting a full
    // interval.
    scheduler::run_check_cycle().await;

    loop {
        let interval_minutes = pallet_core::config::Config::load()
            .map(|c| c.check_interval_minutes)
            .unwrap_or(60)
            .max(1);
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(interval_minutes * 60));
        tokio::pin!(sleep);

        tokio::select! {
            _ = &mut sleep => {
                scheduler::run_check_cycle().await;
            }
            Some(()) = recv_fs_event(&mut fs_events) => {
                if last_fs_triggered_check.elapsed() < MIN_FS_TRIGGER_GAP {
                    tracing::debug!("ignoring filesystem trigger: still within the cooldown window");
                    continue;
                }
                last_fs_triggered_check = std::time::Instant::now();
                info!("filesystem watch triggered an out-of-cycle check");
                scheduler::run_check_cycle().await;
            }
            _ = shutdown.changed() => {
                return;
            }
        }
    }
}

async fn recv_fs_event(rx: &mut Option<tokio::sync::mpsc::Receiver<()>>) -> Option<()> {
    match rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}
