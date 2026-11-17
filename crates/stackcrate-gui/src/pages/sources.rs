use super::{clear_list, placeholder_row, scaffold, source_badge};
use adw::prelude::*;
use gtk::glib;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Stack {
    let super::PageScaffold {
        root,
        toolbar,
        list,
        ..
    } = scaffold();

    let label = gtk::Label::new(Some("Active sources and pinned packages"));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    toolbar.append(&label);
    toolbar.append(&refresh_button);

    let do_refresh = {
        let list = list.clone();
        let overlay = overlay.clone();
        move || {
            clear_list(&list);
            let list = list.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                let statuses =
                    crate::backend::run_blocking(crate::backend::actions::sources_status).await;
                let disk_usage =
                    crate::backend::run_blocking(crate::backend::actions::disk_usage_summary)
                        .await
                        .unwrap_or_default();
                let pins = crate::backend::run_blocking(crate::backend::actions::list_pins).await;

                clear_list(&list);

                let header = adw::ActionRow::builder()
                    .title("Sources")
                    .selectable(false)
                    .activatable(false)
                    .build();
                header.add_css_class("dim-label");
                list.append(&header);

                for status in statuses.unwrap_or_default() {
                    let usage = disk_usage.iter().find(|e| e.source == status.source);
                    list.append(&build_source_row(&overlay, status, usage));
                }

                let pins_header = adw::ActionRow::builder()
                    .title("Pinned packages")
                    .subtitle("Held back from bulk updates")
                    .selectable(false)
                    .activatable(false)
                    .build();
                pins_header.add_css_class("dim-label");
                list.append(&pins_header);

                let pins = pins.unwrap_or_default();
                if pins.is_empty() {
                    list.append(&placeholder_row(
                        "view-pin-symbolic",
                        "Nothing pinned",
                        "Pin a package from its detail view to hold it back from bulk updates.",
                    ));
                } else {
                    for pin in pins {
                        list.append(&build_pin_row(&overlay, pin));
                    }
                }

                let diagnostics_header = adw::ActionRow::builder()
                    .title("Diagnostics")
                    .selectable(false)
                    .activatable(false)
                    .build();
                diagnostics_header.add_css_class("dim-label");
                list.append(&diagnostics_header);

                let checks = crate::backend::run_blocking(crate::backend::actions::doctor)
                    .await
                    .unwrap_or_default();
                for check in checks {
                    list.append(&build_doctor_row(&check));
                }
            });
        }
    };

    {
        let do_refresh = do_refresh.clone();
        refresh_button.connect_clicked(move |_| do_refresh());
    }
    do_refresh();

    root
}

fn build_source_row(
    overlay: &adw::ToastOverlay,
    status: crate::backend::actions::SourceStatus,
    usage: Option<&stackcrate_core::types::DiskUsageEntry>,
) -> adw::ActionRow {
    let mut subtitle = if !status.available {
        "Not installed on this system".to_string()
    } else if status.enabled {
        "Available and enabled".to_string()
    } else {
        "Disabled".to_string()
    };
    if let Some(usage) = usage.filter(|u| u.package_count > 0) {
        subtitle = format!(
            "{subtitle} · {} across {} package(s)",
            stackcrate_core::types::format_size_kb(usage.total_kb),
            usage.package_count
        );
    }
    let row = adw::ActionRow::builder()
        .title(super::source_badge_text(status.source))
        .subtitle(subtitle)
        .build();
    row.add_prefix(&source_badge(status.source));

    let toggle = gtk::Switch::new();
    toggle.set_active(status.enabled);
    toggle.set_sensitive(status.available);
    toggle.set_valign(gtk::Align::Center);
    let overlay = overlay.clone();
    toggle.connect_state_set(move |_, enabled| {
        crate::backend::actions::set_source_enabled(status.source, enabled);
        super::toast(
            &overlay,
            format!(
                "{} {}",
                super::source_badge_text(status.source),
                if enabled { "enabled" } else { "disabled" }
            ),
        );
        glib::Propagation::Proceed
    });
    row.add_suffix(&toggle);
    row.set_activatable_widget(Some(&toggle));

    row
}

fn build_pin_row(
    overlay: &adw::ToastOverlay,
    pin: stackcrate_core::registry::PinInfo,
) -> adw::ActionRow {
    let subtitle = match &pin.pinned_version {
        Some(v) => format!("{} · pinned to {v}", super::source_badge_text(pin.source)),
        None => super::source_badge_text(pin.source).to_string(),
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pin.package_id).to_string())
        .subtitle(subtitle)
        .build();
    row.add_prefix(&source_badge(pin.source));

    let unpin_button = gtk::Button::with_label("Unpin");
    unpin_button.set_valign(gtk::Align::Center);
    let overlay = overlay.clone();
    let row_weak = row.downgrade();
    let source = pin.source;
    let id = pin.package_id.clone();
    unpin_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        let overlay = overlay.clone();
        let row_weak = row_weak.clone();
        let source = source;
        let id = id.clone();
        glib::spawn_future_local(async move {
            let outcome =
                crate::backend::run_blocking(move || crate::backend::actions::unpin(source, id))
                    .await;
            if let Some(outcome) = outcome {
                super::toast(&overlay, outcome.summary);
                if outcome.success
                    && let Some(row) = row_weak.upgrade()
                {
                    row.set_visible(false);
                }
            }
        });
    });
    row.add_suffix(&unpin_button);

    row
}

fn build_doctor_row(check: &stackcrate_core::DoctorCheck) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&check.name).to_string())
        .subtitle(glib::markup_escape_text(&check.detail).to_string())
        .build();
    let icon = gtk::Image::from_icon_name(if check.ok {
        "emblem-ok-symbolic"
    } else {
        "dialog-warning-symbolic"
    });
    icon.add_css_class(if check.ok { "success" } else { "warning" });
    row.add_prefix(&icon);
    row
}
