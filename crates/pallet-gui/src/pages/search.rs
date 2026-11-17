use super::{PageScaffold, clear_list, scaffold, source_badge_text, toast};
use adw::prelude::*;
use gtk::glib;
use pallet_core::types::SourceId;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Box {
    let PageScaffold {
        root,
        toolbar,
        list,
    } = scaffold();

    let entry = gtk::SearchEntry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Search apt, Flatpak, Snap..."));
    let search_button = gtk::Button::with_label("Search");
    toolbar.append(&entry);
    toolbar.append(&search_button);

    let run_search = {
        let list = list.clone();
        let overlay = overlay.clone();
        let entry = entry.clone();
        move || {
            let query = entry.text().to_string();
            if query.trim().is_empty() {
                return;
            }
            clear_list(&list);
            let placeholder = adw::ActionRow::builder().title("Searching...").build();
            list.append(&placeholder);

            let list = list.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                let results = crate::backend::run_blocking(move || {
                    crate::backend::actions::search_all(query)
                })
                .await;
                clear_list(&list);
                match results {
                    Some(results) if !results.is_empty() => {
                        for pkg in results {
                            list.append(&build_result_row(&overlay, &pkg));
                        }
                    }
                    Some(_) => {
                        list.append(&adw::ActionRow::builder().title("No results").build());
                    }
                    None => toast(&overlay, "Search failed"),
                }
            });
        }
    };

    {
        let run_search = run_search.clone();
        search_button.connect_clicked(move |_| run_search());
    }
    entry.connect_activate(move |_| run_search());

    root
}

fn build_result_row(
    overlay: &adw::ToastOverlay,
    pkg: &pallet_core::types::Package,
) -> adw::ActionRow {
    let source = pkg.source.unwrap_or(SourceId::Apt);
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pkg.name).to_string())
        .subtitle(glib::markup_escape_text(&pkg.description).to_string())
        .build();

    let badge = gtk::Label::new(Some(source_badge_text(source)));
    badge.add_css_class("dim-label");
    badge.set_margin_end(8);
    row.add_prefix(&badge);

    let install_button = gtk::Button::with_label("Install");
    install_button.set_valign(gtk::Align::Center);
    let id = pkg.id.clone();
    let overlay = overlay.clone();
    let row_weak = row.downgrade();
    install_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Installing...");
        let id = id.clone();
        let overlay = overlay.clone();
        let button = button.clone();
        let row_weak = row_weak.clone();
        glib::spawn_future_local(async move {
            let outcome = crate::backend::run_blocking(move || {
                crate::backend::actions::install_named(source, id)
            })
            .await;
            match outcome {
                Some(outcome) if outcome.success => {
                    toast(&overlay, outcome.summary);
                    button.set_label("Installed");
                    if let Some(row) = row_weak.upgrade() {
                        row.set_sensitive(false);
                    }
                }
                Some(outcome) => {
                    toast(&overlay, outcome.summary);
                    button.set_label("Install");
                    button.set_sensitive(true);
                }
                None => {
                    toast(&overlay, "Install failed unexpectedly");
                    button.set_label("Install");
                    button.set_sensitive(true);
                }
            }
        });
    });
    row.add_suffix(&install_button);
    row
}
