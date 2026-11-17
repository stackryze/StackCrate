//! The periodic "check for updates" cycle: soft (unprivileged) checks always
//! run on the timer; a privileged apt metadata refresh only runs when it can
//! happen without an auth prompt (see `passwordless_available`); update
//! policy (notify/security-only/auto-all) is applied after each check.

use stackcrate_core::config::UpdatePolicy;
use stackcrate_core::types::{OperationOutcome, UpdateKind, fingerprint_updates};
use stackcrate_core::{Engine, privilege};
use tracing::{info, warn};

pub async fn run_check_cycle() {
    if let Err(e) = tokio::task::spawn_blocking(check_cycle_blocking).await {
        warn!("check cycle task panicked: {e}");
    }
}

pub(crate) fn check_cycle_blocking() {
    let engine = match Engine::open() {
        Ok(e) => e,
        Err(e) => {
            warn!("could not open engine for scheduled check: {e}");
            return;
        }
    };

    if privilege::passwordless_available("apt-get") {
        match engine.refresh_metadata() {
            Ok(outcomes) => {
                for outcome in outcomes {
                    if !outcome.success {
                        warn!("apt metadata refresh failed: {}", outcome.log);
                    }
                }
            }
            Err(e) => warn!("apt metadata refresh failed: {e}"),
        }
    } else {
        info!(
            "skipping privileged metadata refresh (no passwordless sudo rule installed; run `crate setup sudoers`)"
        );
    }

    let updates = match engine.check_updates_all() {
        Ok(u) => u,
        Err(e) => {
            warn!("update check failed: {e}");
            return;
        }
    };
    info!("checked all sources: {} update(s) pending", updates.len());

    apply_policy(&engine, &updates);
    maybe_notify(&engine, &updates);
}

fn apply_policy(engine: &Engine, updates: &[stackcrate_core::types::UpdateInfo]) {
    let pinned: std::collections::HashSet<(stackcrate_core::types::SourceId, String)> = engine
        .registry
        .list_pins()
        .unwrap_or_default()
        .into_iter()
        .map(|p| (p.source, p.package_id))
        .collect();
    let actionable: Vec<_> = updates
        .iter()
        .filter(|u| !pinned.contains(&(u.source, u.id.clone())))
        .collect();

    if engine.config.update_policy != UpdatePolicy::NotifyOnly
        && engine.config.require_ac_power_for_auto_update
        && stackcrate_core::power::on_ac_power() == Some(false)
    {
        info!(
            "on battery power with require-ac-power-for-auto-update set: skipping unattended \
             install this cycle (still notifying as usual)"
        );
        return;
    }

    match engine.config.update_policy {
        UpdatePolicy::NotifyOnly => {}
        UpdatePolicy::SecurityOnly => {
            let apt_ok = privilege::passwordless_available("apt-get");
            for update in actionable.iter().filter(|u| u.kind == UpdateKind::Security) {
                if update.source == stackcrate_core::types::SourceId::Apt && !apt_ok {
                    continue;
                }
                apply_one(engine, update.source, &update.id);
            }
        }
        UpdatePolicy::AutoAll => {
            let apt_ok = privilege::passwordless_available("apt-get");
            for source in engine.active_sources() {
                if source.id() == stackcrate_core::types::SourceId::Apt && !apt_ok {
                    info!("skipping unattended apt upgrade (no passwordless sudo rule installed)");
                    continue;
                }
                if !actionable.iter().any(|u| u.source == source.id()) {
                    continue;
                }
                match source.update(None) {
                    Ok(outcome) => log_outcome(engine, source.id(), "(all)", &outcome),
                    Err(e) => warn!("{}: unattended update failed: {e}", source.id()),
                }
            }
        }
    }
}

fn apply_one(engine: &Engine, source: stackcrate_core::types::SourceId, id: &str) {
    match stackcrate_core::sources::source_by_id(source).update(Some(id)) {
        Ok(outcome) => log_outcome(engine, source, id, &outcome),
        Err(e) => warn!("{source}: unattended update of {id} failed: {e}"),
    }
}

fn log_outcome(
    engine: &Engine,
    source: stackcrate_core::types::SourceId,
    package: &str,
    outcome: &OperationOutcome,
) {
    let _ = engine
        .registry
        .log_history(&stackcrate_core::registry::HistoryEntry {
            timestamp: chrono::Utc::now(),
            source: source.as_str().to_string(),
            action: "auto-update".to_string(),
            package: package.to_string(),
            success: outcome.success,
            detail: outcome.summary.clone(),
        });
}

fn maybe_notify(engine: &Engine, updates: &[stackcrate_core::types::UpdateInfo]) {
    if !engine.config.notifications_enabled || updates.is_empty() {
        return;
    }
    let fingerprint = fingerprint_updates(updates);
    let already_notified = engine
        .registry
        .last_notified_fingerprint()
        .ok()
        .flatten()
        .map(|last| last == fingerprint)
        .unwrap_or(false);
    if already_notified {
        return;
    }
    let security = updates
        .iter()
        .filter(|u| u.kind == UpdateKind::Security)
        .count();
    crate::notify::send_update_notification(updates.len(), security);
    let _ = engine.registry.set_last_notified_fingerprint(&fingerprint);
}
