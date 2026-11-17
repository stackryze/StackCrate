use adw::prelude::*;
use gtk::glib;
use pallet_core::config::{Config, UpdatePolicy};

pub fn open(parent: &adw::ApplicationWindow) {
    let config = Config::load().unwrap_or_default();

    let window = adw::PreferencesWindow::builder()
        .transient_for(parent)
        .modal(true)
        .title("Pallet Settings")
        .build();

    let page = adw::PreferencesPage::new();
    let group = adw::PreferencesGroup::builder()
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
    group.add(&interval_row);

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
    group.add(&policy_row);

    let notifications_row = adw::ActionRow::builder()
        .title("Desktop notifications")
        .subtitle("Show a notification when updates are found")
        .build();
    let notifications_switch = gtk::Switch::new();
    notifications_switch.set_active(config.notifications_enabled);
    notifications_switch.set_valign(gtk::Align::Center);
    notifications_row.add_suffix(&notifications_switch);
    notifications_row.set_activatable_widget(Some(&notifications_switch));
    group.add(&notifications_row);

    page.add(&group);
    window.add(&page);

    let save = {
        let interval_spin = interval_spin.clone();
        let policy_row = policy_row.clone();
        let notifications_switch = notifications_switch.clone();
        move || {
            let mut config = Config::load().unwrap_or_default();
            config.check_interval_minutes = interval_spin.value() as u64;
            config.update_policy = match policy_row.selected() {
                1 => UpdatePolicy::SecurityOnly,
                2 => UpdatePolicy::AutoAll,
                _ => UpdatePolicy::NotifyOnly,
            };
            config.notifications_enabled = notifications_switch.is_active();
            let _ = config.save();
        }
    };

    window.connect_close_request(move |_| {
        save();
        glib::Propagation::Proceed
    });

    window.present();
}
