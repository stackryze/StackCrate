use super::{PageScaffold, clear_list, scaffold, source_badge_text, toast};
use adw::prelude::*;
use gtk::glib;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Box {
    let PageScaffold {
        root,
        toolbar,
        list,
    } = scaffold();

    let refresh_button = gtk::Button::with_label("Refresh");
    let label = gtk::Label::new(Some("Installed packages across all active sources"));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    toolbar.append(&label);
    toolbar.append(&refresh_button);

    let do_refresh = {
        let list = list.clone();
        let overlay = overlay.clone();
        move || {
            clear_list(&list);
            list.append(&adw::ActionRow::builder().title("Loading...").build());
            let list = list.clone();
            let overlay = overlay.clone();
            glib::spawn_future_local(async move {
                let packages =
                    crate::backend::run_blocking(crate::backend::actions::list_installed_all).await;
                clear_list(&list);
                match packages {
                    Some(packages) if !packages.is_empty() => {
                        for pkg in packages {
                            list.append(&build_row(&overlay, &pkg));
                        }
                    }
                    Some(_) => {
                        list.append(
                            &adw::ActionRow::builder()
                                .title("Nothing installed yet")
                                .build(),
                        );
                    }
                    None => toast(&overlay, "Could not load installed packages"),
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

fn build_row(overlay: &adw::ToastOverlay, pkg: &stackcrate_core::types::Package) -> adw::ActionRow {
    let source = pkg.source.unwrap_or(stackcrate_core::types::SourceId::Apt);
    let subtitle = if pkg.version.is_empty() {
        pkg.description.clone()
    } else {
        format!("{}  ·  {}", pkg.version, pkg.description)
    };
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pkg.name).to_string())
        .subtitle(glib::markup_escape_text(&subtitle).to_string())
        .build();

    let badge = gtk::Label::new(Some(source_badge_text(source)));
    badge.add_css_class("dim-label");
    badge.set_margin_end(8);
    row.add_prefix(&badge);

    let remove_button = gtk::Button::with_label("Remove");
    remove_button.add_css_class("destructive-action");
    remove_button.set_valign(gtk::Align::Center);
    let id = pkg.id.clone();
    let overlay = overlay.clone();
    let row_weak = row.downgrade();
    remove_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Removing...");
        let id = id.clone();
        let overlay = overlay.clone();
        let button = button.clone();
        let row_weak = row_weak.clone();
        glib::spawn_future_local(async move {
            let outcome =
                crate::backend::run_blocking(move || crate::backend::actions::remove(source, id))
                    .await;
            match outcome {
                Some(outcome) => {
                    toast(&overlay, outcome.summary.clone());
                    if outcome.success {
                        if let Some(row) = row_weak.upgrade() {
                            row.set_visible(false);
                        }
                    } else {
                        button.set_label("Remove");
                        button.set_sensitive(true);
                    }
                }
                None => {
                    toast(&overlay, "Remove failed unexpectedly");
                    button.set_label("Remove");
                    button.set_sensitive(true);
                }
            }
        });
    });
    row.add_suffix(&remove_button);
    row
}
