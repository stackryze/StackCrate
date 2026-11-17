//! Settings window: grouped into General (background monitor), Updates
//! (policy + power-aware auto-update), Checksums (URL install safety), and
//! Advanced (one-time local setup StackCrate's CLI already offers via
//! `crate setup sudoers`/`crate setup daemon`, exposed here too so a
//! GUI-only user never has to touch a terminal for the one-time bits).

use adw::prelude::*;
use gtk::glib;
use stackcrate_core::config::{Config, UpdatePolicy};

use crate::pages::toast;

pub fn open(parent: &adw::ApplicationWindow, overlay: &adw::ToastOverlay) {
    let config = Config::load().unwrap_or_default();

    let window = adw::PreferencesDialog::builder()
        .title("StackCrate Settings")
        .search_enabled(false)
        .build();

    let page = adw::PreferencesPage::new();

    // ---- General: background monitor -----------------------------------
    let monitor_group = adw::PreferencesGroup::builder()
        .title("Background Monitor")
        .build();

    let interval_row = adw::ActionRow::builder()
        .title("Check interval (minutes)")
        .subtitle("How often the background daemon checks for updates")
        .build();
    let interval_spin = gtk::SpinButton::with_range(5.0, 1440.0, 5.0);
    interval_spin.set_value(config.check_interval_minutes as f64);
    interval_spin.set_valign(gtk::Align::Center);
    interval_row.add_suffix(&interval_spin);
    monitor_group.add(&interval_row);

    let notifications_row = adw::ActionRow::builder()
        .title("Desktop notifications")
        .subtitle("Show a notification when updates are found")
        .build();
    let notifications_switch = gtk::Switch::new();
    notifications_switch.set_active(config.notifications_enabled);
    notifications_switch.set_valign(gtk::Align::Center);
    notifications_row.add_suffix(&notifications_switch);
    notifications_row.set_activatable_widget(Some(&notifications_switch));
    monitor_group.add(&notifications_row);
    page.add(&monitor_group);

    // ---- Updates: policy + power awareness ------------------------------
    let updates_group = adw::PreferencesGroup::builder().title("Updates").build();

    let policy_row = adw::ComboRow::builder()
        .title("Update policy")
        .subtitle("What happens automatically when updates are found")
        .build();
    let policy_model = gtk::StringList::new(&[
        "Notify only",
        "Auto-install security updates",
        "Auto-install everything",
    ]);
    policy_row.set_model(Some(&policy_model));
    policy_row.set_selected(match config.update_policy {
        UpdatePolicy::NotifyOnly => 0,
        UpdatePolicy::SecurityOnly => 1,
        UpdatePolicy::AutoAll => 2,
    });
    updates_group.add(&policy_row);

    let ac_power_row = adw::ActionRow::builder()
        .title("Only auto-install on AC power")
        .subtitle("On battery, fall back to just notifying until plugged in")
        .build();
    let ac_power_switch = gtk::Switch::new();
    ac_power_switch.set_active(config.require_ac_power_for_auto_update);
    ac_power_switch.set_valign(gtk::Align::Center);
    ac_power_row.add_suffix(&ac_power_switch);
    ac_power_row.set_activatable_widget(Some(&ac_power_switch));
    updates_group.add(&ac_power_row);
    page.add(&updates_group);

    // ---- Checksums: URL install safety -----------------------------------
    let checksum_group = adw::PreferencesGroup::builder()
        .title("Checksums")
        .description(
            "Require a sha256/sha512 checksum when installing an AppImage or archive from a URL",
        )
        .build();

    let require_appimage_row = adw::ActionRow::builder()
        .title("Require for AppImage URLs")
        .build();
    let require_appimage_switch = gtk::Switch::new();
    require_appimage_switch.set_active(config.require_checksum_for_appimage);
    require_appimage_switch.set_valign(gtk::Align::Center);
    require_appimage_row.add_suffix(&require_appimage_switch);
    require_appimage_row.set_activatable_widget(Some(&require_appimage_switch));
    checksum_group.add(&require_appimage_row);

    let require_archive_row = adw::ActionRow::builder()
        .title("Require for archive URLs")
        .build();
    let require_archive_switch = gtk::Switch::new();
    require_archive_switch.set_active(config.require_checksum_for_archive);
    require_archive_switch.set_valign(gtk::Align::Center);
    require_archive_row.add_suffix(&require_archive_switch);
    require_archive_row.set_activatable_widget(Some(&require_archive_switch));
    checksum_group.add(&require_archive_row);
    page.add(&checksum_group);

    // ---- Advanced: one-time local setup -----------------------------------
    let advanced_group = adw::PreferencesGroup::builder()
        .title("Advanced")
        .description("One-time local setup; the same as `crate setup sudoers`/`crate setup daemon`")
        .build();

    let sudoers_row = adw::ActionRow::builder()
        .title("Passwordless background updates")
        .subtitle(
            "Installs a narrow /etc/sudoers.d/stackcrate rule so scheduled checks and auto-updates \
             don't need a password prompt. Entirely optional; review before accepting.",
        )
        .build();
    let sudoers_button = gtk::Button::with_label("Set Up…");
    sudoers_button.set_valign(gtk::Align::Center);
    let overlay_for_sudoers = overlay.clone();
    sudoers_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        let overlay = overlay_for_sudoers.clone();
        let button = button.clone();
        glib::spawn_future_local(async move {
            let outcome = crate::backend::run_blocking(setup_sudoers_blocking).await;
            match outcome {
                Some(Ok(())) => toast(&overlay, "Sudoers rule installed."),
                Some(Err(e)) => toast(&overlay, format!("Could not install it: {e}")),
                None => toast(&overlay, "Could not install it unexpectedly"),
            }
            button.set_sensitive(true);
        });
    });
    sudoers_row.add_suffix(&sudoers_button);
    advanced_group.add(&sudoers_row);

    let daemon_row = adw::ActionRow::builder()
        .title("Background monitor service")
        .subtitle("Installs and enables the stackcrate-daemon systemd --user service")
        .build();
    let daemon_button = gtk::Button::with_label("Enable…");
    daemon_button.set_valign(gtk::Align::Center);
    let overlay_for_daemon = overlay.clone();
    daemon_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        let overlay = overlay_for_daemon.clone();
        let button = button.clone();
        glib::spawn_future_local(async move {
            let outcome = crate::backend::run_blocking(|| {
                stackcrate_core::setup::install_and_start_daemon_unit()
            })
            .await;
            match outcome {
                Some(Ok(_)) => toast(&overlay, "Background monitor enabled and running."),
                Some(Err(e)) => toast(&overlay, format!("Could not enable it: {e}")),
                None => toast(&overlay, "Could not enable it unexpectedly"),
            }
            button.set_sensitive(true);
        });
    });
    daemon_row.add_suffix(&daemon_button);
    advanced_group.add(&daemon_row);
    page.add(&advanced_group);

    window.add(&page);

    let save = {
        let interval_spin = interval_spin.clone();
        let policy_row = policy_row.clone();
        let notifications_switch = notifications_switch.clone();
        let ac_power_switch = ac_power_switch.clone();
        let require_appimage_switch = require_appimage_switch.clone();
        let require_archive_switch = require_archive_switch.clone();
        move || {
            let mut config = Config::load().unwrap_or_default();
            config.check_interval_minutes = interval_spin.value() as u64;
            config.update_policy = match policy_row.selected() {
                1 => UpdatePolicy::SecurityOnly,
                2 => UpdatePolicy::AutoAll,
                _ => UpdatePolicy::NotifyOnly,
            };
            config.notifications_enabled = notifications_switch.is_active();
            config.require_ac_power_for_auto_update = ac_power_switch.is_active();
            config.require_checksum_for_appimage = require_appimage_switch.is_active();
            config.require_checksum_for_archive = require_archive_switch.is_active();
            let _ = config.save();
        }
    };

    window.connect_closed(move |_| {
        save();
    });

    window.present(Some(parent));
}

fn setup_sudoers_blocking() -> stackcrate_core::Result<()> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .map_err(|_| stackcrate_core::Error::other("could not determine the current username"))?;
    stackcrate_core::privilege::install_sudoers_rule(&user)?;
    Ok(())
}
