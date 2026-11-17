use super::{
    PageScaffold, clear_list, filters, loading_row, placeholder_row, scaffold, source_badge, toast,
};
use adw::prelude::*;
use gtk::glib;
use stackcrate_core::types::{Package, SourceId};
use std::cell::RefCell;
use std::rc::Rc;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Stack {
    let PageScaffold {
        root,
        list_page,
        toolbar,
        list,
    } = scaffold();
    let checked: Rc<RefCell<Vec<(SourceId, String, gtk::CheckButton)>>> =
        Rc::new(RefCell::new(Vec::new()));
    let all_results: Rc<RefCell<Vec<Package>>> = Rc::new(RefCell::new(Vec::new()));

    let entry = gtk::SearchEntry::new();
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Search apt, Flatpak, Snap..."));
    let search_button = gtk::Button::with_label("Search");
    let install_selected_button = gtk::Button::with_label("Install Selected");
    install_selected_button.add_css_class("suggested-action");
    install_selected_button.set_visible(false);
    toolbar.append(&entry);
    toolbar.append(&search_button);
    toolbar.append(&install_selected_button);

    // --- filter/sort row (applies to the already-fetched result set) ---
    let filter_bar = filters::filter_row();
    let source_filter = filters::source_filter(SourceId::all());
    let installed_filter = gtk::DropDown::builder()
        .model(&gtk::StringList::new(&[
            "All",
            "Installed only",
            "Not installed",
        ]))
        .build();
    let sort_dropdown = filters::sort_dropdown(&["Relevance", "Name (A-Z)", "Name (Z-A)"]);
    filter_bar.append(&source_filter.widget);
    filter_bar.append(&installed_filter);
    filter_bar.append(&sort_dropdown);
    list_page.insert_child_after(&filter_bar, Some(&toolbar));

    let do_render: Rc<dyn Fn()> = {
        let list = list.clone();
        let overlay = overlay.clone();
        let root = root.clone();
        let checked = checked.clone();
        let all_results = all_results.clone();
        let source_filter = source_filter.clone();
        let installed_filter = installed_filter.clone();
        let sort_dropdown = sort_dropdown.clone();
        let install_selected_button = install_selected_button.clone();
        Rc::new(move || {
            let mut filtered: Vec<Package> = all_results
                .borrow()
                .iter()
                .filter(|pkg| source_filter.is_active(pkg.source.unwrap_or(SourceId::Apt)))
                .filter(|pkg| match installed_filter.selected() {
                    1 => pkg.installed,
                    2 => !pkg.installed,
                    _ => true,
                })
                .cloned()
                .collect();

            match sort_dropdown.selected() {
                1 => filtered.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                }),
                2 => filtered.sort_by(|a, b| {
                    b.name
                        .to_ascii_lowercase()
                        .cmp(&a.name.to_ascii_lowercase())
                }),
                _ => {} // "Relevance": keep the already-ranked order from the engine
            }

            clear_list(&list);
            checked.borrow_mut().clear();
            if filtered.is_empty() {
                install_selected_button.set_visible(false);
                let (title, subtitle) = if all_results.borrow().is_empty() {
                    ("Search for software", "Try a package name above.")
                } else {
                    ("No matches", "Try a different filter.")
                };
                list.append(&placeholder_row("system-search-symbolic", title, subtitle));
            } else {
                install_selected_button.set_visible(true);
                for pkg in &filtered {
                    let (row, checkbox) = build_result_row(&overlay, &root, pkg);
                    checked.borrow_mut().push((
                        pkg.source.unwrap_or(SourceId::Apt),
                        pkg.id.clone(),
                        checkbox,
                    ));
                    list.append(&row);
                }
            }
        })
    };

    {
        let do_render = do_render.clone();
        installed_filter.connect_selected_notify(move |_| do_render());
    }
    {
        let do_render = do_render.clone();
        sort_dropdown.connect_selected_notify(move |_| do_render());
    }
    source_filter.connect_changed(do_render.clone());

    let run_search = {
        let list = list.clone();
        let overlay = overlay.clone();
        let entry = entry.clone();
        let all_results = all_results.clone();
        let do_render = do_render.clone();
        move || {
            let query = entry.text().to_string();
            if query.trim().is_empty() {
                return;
            }
            clear_list(&list);
            list.append(&loading_row("Searching…"));

            let overlay = overlay.clone();
            let all_results = all_results.clone();
            let do_render = do_render.clone();
            glib::spawn_future_local(async move {
                let results = crate::backend::run_blocking(move || {
                    crate::backend::actions::search_all(query)
                })
                .await;
                match results {
                    Some(results) => {
                        *all_results.borrow_mut() = results;
                        do_render();
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

    {
        let checked = checked.clone();
        let overlay = overlay.clone();
        install_selected_button.connect_clicked(move |button| {
            let items: Vec<(SourceId, String)> = checked
                .borrow()
                .iter()
                .filter(|(_, _, cb)| cb.is_active())
                .map(|(source, id, _)| (*source, id.clone()))
                .collect();
            if items.is_empty() {
                toast(&overlay, "Select at least one package first");
                return;
            }
            button.set_sensitive(false);
            let overlay = overlay.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let mut ok = 0u32;
                let mut failed = 0u32;
                for (source, id) in items {
                    let outcome = crate::backend::run_blocking(move || {
                        crate::backend::actions::install_named(source, id)
                    })
                    .await;
                    match outcome {
                        Some(outcome) if outcome.success => ok += 1,
                        _ => failed += 1,
                    }
                }
                toast(&overlay, format!("{ok} installed, {failed} failed"));
                button.set_sensitive(true);
            });
        });
    }

    root
}

fn build_result_row(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    pkg: &stackcrate_core::types::Package,
) -> (adw::ActionRow, gtk::CheckButton) {
    let source = pkg.source.unwrap_or(SourceId::Apt);
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pkg.name).to_string())
        .subtitle(glib::markup_escape_text(&pkg.description).to_string())
        .activatable(true)
        .build();

    let checkbox = gtk::CheckButton::new();
    checkbox.set_valign(gtk::Align::Center);
    row.add_prefix(&checkbox);
    row.add_prefix(&source_badge(source));
    if pkg.installed {
        let installed_badge = gtk::Label::new(Some("installed"));
        installed_badge.add_css_class("dim-label");
        row.add_suffix(&installed_badge);
    }

    let install_button = gtk::Button::with_label("Install");
    install_button.set_valign(gtk::Align::Center);
    install_button.set_sensitive(!pkg.installed);
    if pkg.installed {
        install_button.set_label("Installed");
    }
    let id = pkg.id.clone();
    let overlay_for_button = overlay.clone();
    let row_weak = row.downgrade();
    install_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Installing...");
        let id = id.clone();
        let overlay = overlay_for_button.clone();
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

    {
        let overlay = overlay.clone();
        let stack = stack.clone();
        let pkg = pkg.clone();
        row.connect_activated(move |_| {
            let on_changed: Rc<dyn Fn()> = Rc::new(|| {});
            let widget = super::detail::build(&overlay, &stack, pkg.clone(), on_changed);
            super::show_detail(&stack, &widget);
        });
    }

    (row, checkbox)
}
