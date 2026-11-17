use super::{PageScaffold, clear_list, scaffold, source_badge_text, toast};
use adw::prelude::*;
use gtk::glib;
use pallet_core::types::{SourceId, UpdateInfo, UpdateKind};
use std::cell::RefCell;
use std::rc::Rc;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Box {
    let PageScaffold {
        root,
        toolbar,
        list,
    } = scaffold();
    let checked: Rc<RefCell<Vec<(SourceId, String, gtk::CheckButton)>>> =
        Rc::new(RefCell::new(Vec::new()));

    let check_now_button = gtk::Button::with_label("Check Now");
    let update_selected_button = gtk::Button::with_label("Update Selected");
    update_selected_button.add_css_class("suggested-action");
    let summary_label = gtk::Label::new(Some("Loading cached updates..."));
    summary_label.set_hexpand(true);
    summary_label.set_halign(gtk::Align::Start);
    toolbar.append(&summary_label);
    toolbar.append(&check_now_button);
    toolbar.append(&update_selected_button);

    let populate = {
        let list = list.clone();
        let summary_label = summary_label.clone();
        let checked = checked.clone();
        Rc::new(move |updates: Vec<UpdateInfo>| {
            clear_list(&list);
            checked.borrow_mut().clear();
            if updates.is_empty() {
                summary_label.set_text("Everything is up to date");
                list.append(
                    &adw::ActionRow::builder()
                        .title("No pending updates")
                        .build(),
                );
                return;
            }
            let security = updates
                .iter()
                .filter(|u| u.kind == UpdateKind::Security)
                .count();
            summary_label.set_text(&format!("{} update(s), {security} security", updates.len()));
            for update in updates {
                let (row, checkbox) = build_row(&update);
                checked
                    .borrow_mut()
                    .push((update.source, update.id.clone(), checkbox));
                list.append(&row);
            }
        })
    };

    let load_cached = {
        let populate = populate.clone();
        move || {
            let populate = populate.clone();
            glib::spawn_future_local(async move {
                let updates =
                    crate::backend::run_blocking(crate::backend::actions::cached_updates).await;
                populate(updates.unwrap_or_default());
            });
        }
    };
    load_cached();

    {
        let populate = populate.clone();
        let overlay = overlay.clone();
        check_now_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            button.set_label("Checking...");
            let populate = populate.clone();
            let overlay = overlay.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let updates =
                    crate::backend::run_blocking(crate::backend::actions::check_updates_now).await;
                populate(updates.unwrap_or_default());
                toast(&overlay, "Checked all sources");
                button.set_sensitive(true);
                button.set_label("Check Now");
            });
        });
    }

    {
        let checked = checked.clone();
        let overlay = overlay.clone();
        let populate = populate.clone();
        update_selected_button.connect_clicked(move |button| {
            let items: Vec<(SourceId, String)> = checked
                .borrow()
                .iter()
                .filter(|(_, _, cb)| cb.is_active())
                .map(|(source, id, _)| (*source, id.clone()))
                .collect();
            if items.is_empty() {
                toast(&overlay, "Select at least one update first");
                return;
            }
            button.set_sensitive(false);
            button.set_label("Updating...");
            let overlay = overlay.clone();
            let populate = populate.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let results = crate::backend::run_blocking(move || {
                    crate::backend::actions::apply_updates(items)
                })
                .await;
                let (ok, failed) = results.unwrap_or_default().into_iter().fold(
                    (0u32, 0u32),
                    |(ok, failed), (_, _, outcome)| {
                        if outcome.success {
                            (ok + 1, failed)
                        } else {
                            (ok, failed + 1)
                        }
                    },
                );
                toast(&overlay, format!("{ok} succeeded, {failed} failed"));
                button.set_sensitive(true);
                button.set_label("Update Selected");
                let updates =
                    crate::backend::run_blocking(crate::backend::actions::cached_updates).await;
                populate(updates.unwrap_or_default());
            });
        });
    }

    root
}

fn build_row(update: &UpdateInfo) -> (adw::ActionRow, gtk::CheckButton) {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&update.name).to_string())
        .subtitle(format!(
            "{} → {}",
            update.current_version, update.new_version
        ))
        .build();

    let checkbox = gtk::CheckButton::new();
    checkbox.set_valign(gtk::Align::Center);
    row.add_prefix(&checkbox);
    row.set_activatable_widget(Some(&checkbox));

    let badge = gtk::Label::new(Some(source_badge_text(update.source)));
    badge.add_css_class("dim-label");
    row.add_suffix(&badge);

    if update.kind == UpdateKind::Security {
        let security_badge = gtk::Label::new(Some("security"));
        security_badge.add_css_class("error");
        row.add_suffix(&security_badge);
    }

    (row, checkbox)
}
