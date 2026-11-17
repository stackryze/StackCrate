//! Desktop notifications via `notify-rust`. Kept to a tiny surface: the
//! scheduler decides *whether* to notify (policy, fingerprint dedup); this
//! module only knows how to render and send one.

use tracing::warn;

pub fn send_update_notification(total: usize, security: usize) {
    let summary = if security > 0 {
        format!("{total} update(s) available ({security} security)")
    } else {
        format!("{total} update(s) available")
    };
    let body = "Run `crate list updates` or open StackCrate to review and install them.";

    // notify-rust's blocking `show()` makes a D-Bus round-trip; run it on a
    // blocking thread so it never stalls the async scheduler.
    let result = std::thread::spawn(move || {
        notify_rust::Notification::new()
            .appname("StackCrate")
            .summary(&summary)
            .body(body)
            .icon("software-update-available")
            .show()
    })
    .join();

    match result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => warn!("failed to send desktop notification: {e}"),
        Err(_) => warn!("notification thread panicked"),
    }
}
