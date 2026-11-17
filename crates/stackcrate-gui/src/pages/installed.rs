use super::{
    PageScaffold, clear_list, filters, loading_row, placeholder_row, scaffold, source_badge, toast,
};
use adw::prelude::*;
use gtk::glib;
use stackcrate_core::types::{Package, SourceId, format_size_kb};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

pub fn build(overlay: &adw::ToastOverlay) -> gtk::Stack {
    let PageScaffold {
        root,
        list_page,
        toolbar,
        list,
    } = scaffold();

    let label = gtk::Label::new(Some("Installed packages across all active sources"));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    let remove_selected_button = gtk::Button::with_label("Remove Selected");
    remove_selected_button.add_css_class("destructive-action");
    toolbar.append(&label);
    toolbar.append(&refresh_button);
    toolbar.append(&remove_selected_button);

    let banner = adw::Banner::new("");
    banner.set_button_label(Some("Clean Up"));
    list_page.insert_child_after(&banner, Some(&toolbar));

    // --- filter/sort row ---
    let filter_bar = filters::filter_row();
    let text_entry = filters::text_filter("Filter installed packages...");
    let source_filter = filters::source_filter(SourceId::all());
    let sort_dropdown = filters::sort_dropdown(&[
        "Name (A-Z)",
        "Name (Z-A)",
        "Size (largest first)",
        "Size (smallest first)",
    ]);
    let updates_only_label = gtk::Label::new(Some("Updates only"));
    updates_only_label.add_css_class("dim-label");
    let updates_only_switch = gtk::Switch::new();
    updates_only_switch.set_valign(gtk::Align::Center);
    filter_bar.append(&text_entry);
    filter_bar.append(&source_filter.widget);
    filter_bar.append(&sort_dropdown);
    filter_bar.append(&updates_only_label);
    filter_bar.append(&updates_only_switch);
    list_page.insert_child_after(&filter_bar, Some(&banner));

    let all_packages: Rc<RefCell<Vec<Package>>> = Rc::new(RefCell::new(Vec::new()));
    let pending_update_ids: Rc<RefCell<HashSet<(SourceId, String)>>> =
        Rc::new(RefCell::new(HashSet::new()));
    let checked: Rc<RefCell<Vec<(SourceId, String, gtk::CheckButton)>>> =
        Rc::new(RefCell::new(Vec::new()));

    let do_render: Rc<dyn Fn()> = {
        let list = list.clone();
        let overlay = overlay.clone();
        let root = root.clone();
        let all_packages = all_packages.clone();
        let pending_update_ids = pending_update_ids.clone();
        let checked = checked.clone();
        let text_entry = text_entry.clone();
        let source_filter = source_filter.clone();
        let sort_dropdown = sort_dropdown.clone();
        let updates_only_switch = updates_only_switch.clone();
        let label = label.clone();
        Rc::new(move || {
            let query = text_entry.text().to_string();
            let updates_only = updates_only_switch.is_active();
            let mut filtered: Vec<Package> = all_packages
                .borrow()
                .iter()
                .filter(|pkg| source_filter.is_active(pkg.source.unwrap_or(SourceId::Apt)))
                .filter(|pkg| filters::matches_text(&query, &[&pkg.name, &pkg.id]))
                .filter(|pkg| {
                    !updates_only
                        || pending_update_ids
                            .borrow()
                            .contains(&(pkg.source.unwrap_or(SourceId::Apt), pkg.id.clone()))
                })
                .cloned()
                .collect();

            match sort_dropdown.selected() {
                1 => filtered.sort_by(|a, b| {
                    b.name
                        .to_ascii_lowercase()
                        .cmp(&a.name.to_ascii_lowercase())
                }),
                2 => filtered.sort_by_key(|a| std::cmp::Reverse(a.size_kb.unwrap_or(0))),
                3 => filtered.sort_by_key(|a| a.size_kb.unwrap_or(0)),
                _ => filtered.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                }),
            }

            clear_list(&list);
            checked.borrow_mut().clear();
            if filtered.is_empty() {
                let (title, subtitle) = if all_packages.borrow().is_empty() {
                    (
                        "Nothing installed yet",
                        "Packages you install will show up here.",
                    )
                } else {
                    ("No matches", "Try a different filter.")
                };
                list.append(&placeholder_row("folder-symbolic", title, subtitle));
            } else {
                for pkg in &filtered {
                    let (row, checkbox) = build_row(&overlay, &root, pkg);
                    checked.borrow_mut().push((
                        pkg.source.unwrap_or(SourceId::Apt),
                        pkg.id.clone(),
                        checkbox,
                    ));
                    list.append(&row);
                }
            }
            let total = all_packages.borrow().len();
            let shown_size_kb: u64 = filtered.iter().filter_map(|p| p.size_kb).sum();
            label.set_text(&format!(
                "{} of {total} package(s) shown · {}",
                filtered.len(),
                format_size_kb(shown_size_kb)
            ));
        })
    };

    {
        let do_render = do_render.clone();
        text_entry.connect_search_changed(move |_| do_render());
    }
    {
        let do_render = do_render.clone();
        sort_dropdown.connect_selected_notify(move |_| do_render());
    }
    {
        let do_render = do_render.clone();
        updates_only_switch.connect_state_set(move |_, _| {
            do_render();
            glib::Propagation::Proceed
        });
    }
    source_filter.connect_changed(do_render.clone());

    let do_refresh = {
        let list = list.clone();
        let overlay = overlay.clone();
        let banner = banner.clone();
        let all_packages = all_packages.clone();
        let pending_update_ids = pending_update_ids.clone();
        let do_render = do_render.clone();
        move || {
            clear_list(&list);
            list.append(&loading_row("Loading…"));
            let overlay = overlay.clone();
            let banner = banner.clone();
            let all_packages = all_packages.clone();
            let pending_update_ids = pending_update_ids.clone();
            let do_render = do_render.clone();
            glib::spawn_future_local(async move {
                let packages =
                    crate::backend::run_blocking(crate::backend::actions::list_installed_all)
                        .await
                        .unwrap_or_default();
                *all_packages.borrow_mut() = packages;

                let updates = crate::backend::run_blocking(crate::backend::actions::cached_updates)
                    .await
                    .unwrap_or_default();
                *pending_update_ids.borrow_mut() =
                    updates.into_iter().map(|u| (u.source, u.id)).collect();

                do_render();

                let orphans = crate::backend::run_blocking(crate::backend::actions::list_orphans)
                    .await
                    .unwrap_or_default();
                if orphans.is_empty() {
                    banner.set_revealed(false);
                } else {
                    let freed: u64 = orphans.iter().filter_map(|p| p.size_kb).sum();
                    banner.set_title(&format!(
                        "{} package(s) are no longer needed{}",
                        orphans.len(),
                        if freed > 0 {
                            format!(" · frees {}", format_size_kb(freed))
                        } else {
                            String::new()
                        }
                    ));
                    banner.set_revealed(true);
                }
                let _ = overlay;
            });
        }
    };

    {
        let do_refresh = do_refresh.clone();
        refresh_button.connect_clicked(move |_| do_refresh());
    }

    {
        let overlay = overlay.clone();
        let root = root.clone();
        let do_refresh = do_refresh.clone();
        banner.connect_button_clicked(move |banner| {
            let overlay = overlay.clone();
            let root = root.clone();
            let do_refresh = do_refresh.clone();
            let banner = banner.clone();
            glib::spawn_future_local(async move {
                let confirmed = super::confirm(
                    &root,
                    "Remove unneeded packages?",
                    "These were pulled in as dependencies and nothing installed still needs them.",
                    "Remove",
                    true,
                )
                .await;
                if !confirmed {
                    return;
                }
                let dialog = Rc::new(crate::progress::open(&root, "Removing unneeded packages"));
                let (result, lines) =
                    crate::backend::run_streaming(crate::backend::actions::autoremove_streaming);
                drain_lines(lines, dialog_line_sink(&dialog));
                let outcome = result.await;
                if let Some(outcome) = outcome {
                    dialog.finish(outcome.success, &outcome.summary);
                    toast(&overlay, outcome.summary);
                    if outcome.success {
                        banner.set_revealed(false);
                        do_refresh();
                    }
                }
            });
        });
    }

    {
        let checked = checked.clone();
        let overlay = overlay.clone();
        let root = root.clone();
        let do_refresh = do_refresh.clone();
        remove_selected_button.connect_clicked(move |button| {
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
            let overlay = overlay.clone();
            let root_for_confirm = root.clone();
            let do_refresh = do_refresh.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let confirmed = super::confirm(
                    &root_for_confirm,
                    &format!("Remove {} package(s)?", items.len()),
                    "This cannot be undone from here.",
                    "Remove",
                    true,
                )
                .await;
                if !confirmed {
                    return;
                }
                button.set_sensitive(false);
                let mut ok = 0u32;
                let mut failed = 0u32;
                for (source, id) in items {
                    let outcome = crate::backend::run_blocking(move || {
                        crate::backend::actions::remove(source, id)
                    })
                    .await;
                    match outcome {
                        Some(outcome) if outcome.success => ok += 1,
                        _ => failed += 1,
                    }
                }
                toast(&overlay, format!("{ok} removed, {failed} failed"));
                button.set_sensitive(true);
                do_refresh();
            });
        });
    }

    do_refresh();

    root
}

/// Bridges a `ProgressDialog` into the `(StreamKind, &str)` shape
/// `run_streaming`'s line channel carries.
fn dialog_line_sink(
    dialog: &Rc<crate::progress::ProgressDialog>,
) -> impl Fn(stackcrate_core::types::StreamKind, &str) + 'static {
    let dialog = dialog.clone();
    move |stream, line| dialog.append_line(stream, line)
}

/// Spawns a local future that forwards every line from `lines` to `on_line`
/// until the channel closes (the operation finished).
fn drain_lines(
    lines: async_channel::Receiver<stackcrate_core::types::ProgressLine>,
    on_line: impl Fn(stackcrate_core::types::StreamKind, &str) + 'static,
) {
    glib::spawn_future_local(async move {
        while let Ok(progress) = lines.recv().await {
            on_line(progress.stream, &progress.line);
        }
    });
}

fn build_row(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    pkg: &stackcrate_core::types::Package,
) -> (adw::ActionRow, gtk::CheckButton) {
    let source = pkg.source.unwrap_or(stackcrate_core::types::SourceId::Apt);
    let mut subtitle = if pkg.version.is_empty() {
        pkg.description.clone()
    } else {
        format!("{}  ·  {}", pkg.version, pkg.description)
    };
    if let Some(size_kb) = pkg.size_kb {
        subtitle = format!("{subtitle}  ·  {}", format_size_kb(size_kb));
    }
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&pkg.name).to_string())
        .subtitle(glib::markup_escape_text(&subtitle).to_string())
        .activatable(true)
        .build();

    let checkbox = gtk::CheckButton::new();
    checkbox.set_valign(gtk::Align::Center);
    row.add_prefix(&checkbox);
    row.add_prefix(&source_badge(source));

    let remove_button = gtk::Button::with_label("Remove");
    remove_button.add_css_class("destructive-action");
    remove_button.set_valign(gtk::Align::Center);
    let id = pkg.id.clone();
    let overlay_for_button = overlay.clone();
    let row_weak = row.downgrade();
    remove_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        button.set_label("Removing...");
        let id = id.clone();
        let overlay = overlay_for_button.clone();
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
