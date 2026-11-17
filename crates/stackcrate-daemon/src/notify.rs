//! Desktop notifications via `notify-rust`. Kept to a tiny surface: the
//! scheduler decides *whether* to notify (policy, fingerprint dedup); this
//! module only knows how to render, send one, and (best-effort) react to the
//! user clicking it.

use tracing::{debug, warn};

pub fn send_update_notification(total: usize, security: usize) {
    let summary = if security > 0 {
        format!("{total} update(s) available ({security} security)")
    } else {
        format!("{total} update(s) available")
    };
    let body = "Run `crate list updates` or open StackCrate to review and install them.";

    // notify-rust's blocking `show()` makes a D-Bus round-trip, and
    // `wait_for_action` below blocks until the user interacts with it (or it
    // times out/is dismissed), so this all runs on its own thread rather
    // than stalling the async scheduler.
    let result = std::thread::spawn(move || {
        notify_rust::Notification::new()
            .appname("StackCrate")
            .summary(&summary)
            .body(body)
            .icon("software-update-available")
            .action("default", "Open StackCrate")
            .show()
    })
    .join();

    match result {
        Ok(Ok(handle)) => {
            handle.wait_for_action(|action| {
                if action == "default" {
                    open_gui();
                }
            });
        }
        Ok(Err(e)) => warn!("failed to send desktop notification: {e}"),
        Err(_) => warn!("notification thread panicked"),
    }
}

/// Launch the GUI in response to the notification being clicked. Best
/// effort: if it isn't installed (CLI/daemon-only setups are supported) or
/// fails to start, this just logs and moves on -- the notification itself
/// already told the user everything it needs to.
fn open_gui() {
    let Some(path) = stackcrate_core::process::which("stackcrate-gui") else {
        debug!("stackcrate-gui is not on PATH; not opening it for the notification click");
        return;
    };
    if let Err(e) = std::process::Command::new(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        warn!("failed to launch stackcrate-gui from the notification: {e}");
    }
}
