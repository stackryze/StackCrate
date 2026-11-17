use super::{
    PageScaffold, clear_list, filters, loading_row, placeholder_row, scaffold, source_badge, toast,
};
use adw::prelude::*;
use gtk::glib;
use stackcrate_core::types::{SourceId, UpdateInfo, UpdateKind};
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
    let all_updates: Rc<RefCell<Vec<UpdateInfo>>> = Rc::new(RefCell::new(Vec::new()));

    let check_now_button = gtk::Button::with_label("Check Now");
    let update_selected_button = gtk::Button::with_label("Update Selected");
    update_selected_button.add_css_class("suggested-action");
    let summary_label = gtk::Label::new(Some("Loading cached updates..."));
    summary_label.set_hexpand(true);
    summary_label.set_halign(gtk::Align::Start);
    toolbar.append(&summary_label);
    toolbar.append(&check_now_button);
    toolbar.append(&update_selected_button);

    // --- filter/sort row ---
    let filter_bar = filters::filter_row();
    let text_entry = filters::text_filter("Filter updates...");
    let source_filter = filters::source_filter(SourceId::all());
    let sort_dropdown = filters::sort_dropdown(&["Security first", "Name (A-Z)"]);
    let security_only_label = gtk::Label::new(Some("Security only"));
    security_only_label.add_css_class("dim-label");
    let security_only_switch = gtk::Switch::new();
    security_only_switch.set_valign(gtk::Align::Center);
    filter_bar.append(&text_entry);
    filter_bar.append(&source_filter.widget);
    filter_bar.append(&sort_dropdown);
    filter_bar.append(&security_only_label);
    filter_bar.append(&security_only_switch);
    list_page.insert_child_after(&filter_bar, Some(&toolbar));

    let do_render: Rc<dyn Fn()> = {
        let list = list.clone();
        let root = root.clone();
        let overlay = overlay.clone();
        let checked = checked.clone();
        let all_updates = all_updates.clone();
        let text_entry = text_entry.clone();
        let source_filter = source_filter.clone();
        let sort_dropdown = sort_dropdown.clone();
        let security_only_switch = security_only_switch.clone();
        let summary_label = summary_label.clone();
        Rc::new(move || {
            let query = text_entry.text().to_string();
            let security_only = security_only_switch.is_active();
            let mut filtered: Vec<UpdateInfo> = all_updates
                .borrow()
                .iter()
                .filter(|u| source_filter.is_active(u.source))
                .filter(|u| filters::matches_text(&query, &[&u.name, &u.id]))
                .filter(|u| !security_only || u.kind == UpdateKind::Security)
                .cloned()
                .collect();

            match sort_dropdown.selected() {
                1 => filtered.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                }),
                _ => filtered.sort_by(|a, b| {
                    let rank = |u: &UpdateInfo| if u.kind == UpdateKind::Security { 0 } else { 1 };
                    rank(a).cmp(&rank(b)).then_with(|| {
                        a.name
                            .to_ascii_lowercase()
                            .cmp(&b.name.to_ascii_lowercase())
                    })
                }),
            }

            clear_list(&list);
            checked.borrow_mut().clear();
            let total = all_updates.borrow().len();
            let total_security = all_updates
                .borrow()
                .iter()
                .filter(|u| u.kind == UpdateKind::Security)
                .count();
            if total == 0 {
                summary_label.set_text("Everything is up to date");
                list.append(&placeholder_row(
                    "emblem-ok-symbolic",
                    "Everything is up to date",
                    "",
                ));
            } else if filtered.is_empty() {
                summary_label.set_text(&format!("{total} update(s), {total_security} security"));
                list.append(&placeholder_row(
                    "software-update-available-symbolic",
                    "No matches",
                    "Try a different filter.",
                ));
            } else {
                summary_label.set_text(&format!(
                    "{} of {total} update(s) shown, {total_security} security",
                    filtered.len()
                ));
                for update in &filtered {
                    let (row, checkbox) = build_row(&overlay, &root, update);
                    checked
                        .borrow_mut()
                        .push((update.source, update.id.clone(), checkbox));
                    list.append(&row);
                }
            }
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
        security_only_switch.connect_state_set(move |_, _| {
            do_render();
            glib::Propagation::Proceed
        });
    }
    source_filter.connect_changed(do_render.clone());

    let populate = {
        let all_updates = all_updates.clone();
        let do_render = do_render.clone();
        Rc::new(move |updates: Vec<UpdateInfo>| {
            *all_updates.borrow_mut() = updates;
            do_render();
        })
    };

    let load_cached = {
        let list = list.clone();
        let populate = populate.clone();
        move || {
            clear_list(&list);
            list.append(&loading_row("Loading cached updates…"));
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
        let root = root.clone();
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
            let root = root.clone();
            let populate = populate.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let dialog = Rc::new(crate::progress::open(&root, "Installing updates"));
                let mut ok = 0u32;
                let mut failed = 0u32;
                for (source, id) in items {
                    dialog.append_line(
                        stackcrate_core::types::StreamKind::Stdout,
                        &format!("--- {id} ({source}) ---"),
                    );
                    let dialog_for_sink = dialog.clone();
                    let (result, lines) = crate::backend::run_streaming(move |sink| {
                        crate::backend::actions::update_streaming(source, id, sink)
                    });
                    glib::spawn_future_local(async move {
                        while let Ok(progress) = lines.recv().await {
                            dialog_for_sink.append_line(progress.stream, &progress.line);
                        }
                    });
                    match result.await {
                        Some(outcome) if outcome.success => ok += 1,
                        _ => failed += 1,
                    }
                }
                dialog.finish(failed == 0, &format!("{ok} succeeded, {failed} failed"));
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

fn build_row(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    update: &UpdateInfo,
) -> (adw::ActionRow, gtk::CheckButton) {
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&update.name).to_string())
        .subtitle(format!(
            "{} → {}",
            update.current_version, update.new_version
        ))
        .activatable(true)
        .build();

    let checkbox = gtk::CheckButton::new();
    checkbox.set_valign(gtk::Align::Center);
    row.add_prefix(&checkbox);
    row.add_prefix(&source_badge(update.source));
    row.set_activatable_widget(Some(&checkbox));

    if update.kind == UpdateKind::Security {
        let security_badge = gtk::Label::new(Some("security"));
        security_badge.add_css_class("error");
        row.add_suffix(&security_badge);
    }

    let detail_button = gtk::Button::from_icon_name("view-more-symbolic");
    detail_button.set_valign(gtk::Align::Center);
    detail_button.set_tooltip_text(Some("Details"));
    {
        let overlay = overlay.clone();
        let stack = stack.clone();
        let source = update.source;
        let id = update.id.clone();
        detail_button.connect_clicked(move |_| {
            let overlay = overlay.clone();
            let stack = stack.clone();
            let id = id.clone();
            glib::spawn_future_local(async move {
                let pkg = crate::backend::run_blocking(move || {
                    crate::backend::actions::package_details(source, id)
                })
                .await
                .flatten();
                if let Some(pkg) = pkg {
                    let on_changed: Rc<dyn Fn()> = Rc::new(|| {});
                    let widget = super::detail::build(&overlay, &stack, pkg, on_changed);
                    super::show_detail(&stack, &widget);
                }
            });
        });
    }
    row.add_suffix(&detail_button);

    (row, checkbox)
}
