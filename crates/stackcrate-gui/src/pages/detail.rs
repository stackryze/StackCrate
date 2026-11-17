//! Package detail view: everything `Engine::package_details` /`explain`/
//! `changelog` know about one package, plus the install/remove/update/pin
//! actions relevant to its current state. Pushed into a page's own
//! `gtk::Stack` (see `pages::show_detail`) rather than a separate window, so
//! going back is just switching back to the list, not closing a dialog.

use super::{source_badge, toast};
use adw::prelude::*;
use gtk::glib;
use stackcrate_core::types::{Package, SourceId, format_size_kb};
use std::rc::Rc;

/// `on_changed` is called after any action that could make the list this
/// detail view was opened from stale (install, remove, pin/unpin), so the
/// caller can refresh it; it is *not* called just for going back.
pub fn build(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    pkg: Package,
    on_changed: Rc<dyn Fn()>,
) -> gtk::Widget {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.set_margin_top(6);

    let back_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    back_row.set_margin_start(6);
    back_row.set_margin_bottom(6);
    let back_button = gtk::Button::builder()
        .icon_name("go-previous-symbolic")
        .css_classes(["flat"])
        .tooltip_text("Back")
        .build();
    {
        let stack = stack.clone();
        back_button.connect_clicked(move |_| super::show_list(&stack));
    }
    back_row.append(&back_button);
    root.append(&back_row);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_vexpand(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_margin_start(18);
    content.set_margin_end(18);
    content.set_margin_bottom(24);
    scroller.set_child(Some(&content));
    root.append(&scroller);

    // Heading: name, source badge, and the installed/update-available state.
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let title = gtk::Label::new(Some(&pkg.name));
    title.add_css_class("title-1");
    title.set_halign(gtk::Align::Start);
    heading.append(&title);
    heading.append(&source_badge(pkg.source.unwrap_or(SourceId::Apt)));
    if pkg.installed {
        let badge = gtk::Label::new(Some("installed"));
        badge.add_css_class("dim-label");
        heading.append(&badge);
    }
    content.append(&heading);

    if !pkg.description.is_empty() {
        let description = gtk::Label::new(Some(&pkg.description));
        description.set_halign(gtk::Align::Start);
        description.set_wrap(true);
        description.set_xalign(0.0);
        description.add_css_class("dim-label");
        content.append(&description);
    }

    let info = adw::PreferencesGroup::new();
    if !pkg.version.is_empty() {
        add_info_row(&info, "Version", &pkg.version);
    }
    if let Some(available) = &pkg.available_version {
        add_info_row(
            &info,
            "Update available",
            &format!("{} → {}", pkg.version, available),
        );
    }
    if let Some(size_kb) = pkg.size_kb {
        add_info_row(&info, "Size", &format_size_kb(size_kb));
    }
    if let Some(maintainer) = &pkg.maintainer {
        add_info_row(&info, "Maintainer", maintainer);
    }
    if let Some(license) = &pkg.license {
        add_info_row(&info, "License", license);
    }
    if let Some(homepage) = &pkg.homepage {
        let row = adw::ActionRow::builder()
            .title("Homepage")
            .subtitle(homepage.as_str())
            .activatable(true)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name("adw-external-link-symbolic"));
        let url = homepage.clone();
        row.connect_activated(move |_| {
            gtk::UriLauncher::new(&url).launch(
                None::<&gtk::Window>,
                gtk::gio::Cancellable::NONE,
                |_| {},
            );
        });
        info.add(&row);
    }
    if let Some(manual) = pkg.manually_installed {
        add_info_row(
            &info,
            "Requested",
            if manual {
                "Explicitly, by you"
            } else {
                "No - pulled in as a dependency"
            },
        );
    }
    content.append(&info);

    if matches!(pkg.source, Some(SourceId::Apt) | Some(SourceId::DebFile)) {
        content.append(&build_explain_expander(
            pkg.id.clone(),
            pkg.source.unwrap_or(SourceId::Apt),
        ));
        content.append(&build_changelog_expander(pkg.id.clone()));
    }
    if matches!(
        pkg.source,
        Some(SourceId::Archive) | Some(SourceId::AppImage)
    ) && pkg.installed
    {
        content.append(&build_rollback_row(
            overlay,
            stack,
            &pkg,
            on_changed.clone(),
        ));
    }

    content.append(&build_actions(overlay, stack, &pkg, on_changed));

    root.upcast::<gtk::Widget>()
}

fn add_info_row(group: &adw::PreferencesGroup, title: &str, value: &str) {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(glib::markup_escape_text(value).to_string())
        .build();
    group.add(&row);
}

fn build_explain_expander(id: String, source: SourceId) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::builder()
        .title("Why is this installed?")
        .subtitle("Click to check (apt only)")
        .build();
    let loaded = std::cell::Cell::new(false);
    expander.connect_notify_local(Some("expanded"), move |row, _| {
        if !row.is_expanded() || loaded.get() {
            return;
        }
        loaded.set(true);
        let id = id.clone();
        let row_weak = row.downgrade();
        glib::spawn_future_local(async move {
            let info =
                crate::backend::run_blocking(move || crate::backend::actions::explain(source, id))
                    .await
                    .flatten();
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            let Some(info) = info else {
                row.set_subtitle("Could not determine this");
                return;
            };
            row.set_subtitle(match info.manually_installed {
                Some(true) => "Explicitly requested",
                Some(false) => "Installed automatically as a dependency",
                None => "Unknown",
            });
            if info.required_by.is_empty() {
                row.add_row(
                    &adw::ActionRow::builder()
                        .title("Nothing currently depends on it")
                        .build(),
                );
            } else {
                for dep in info.required_by {
                    row.add_row(&adw::ActionRow::builder().title(dep).build());
                }
            }
        });
    });
    expander
}

fn build_changelog_expander(id: String) -> adw::ExpanderRow {
    let expander = adw::ExpanderRow::builder()
        .title("Changelog")
        .subtitle("Click to load (needs network access)")
        .build();
    let loaded = std::cell::Cell::new(false);
    expander.connect_notify_local(Some("expanded"), move |row, _| {
        if !row.is_expanded() || loaded.get() {
            return;
        }
        loaded.set(true);
        row.set_subtitle("Loading…");
        let id = id.clone();
        let row_weak = row.downgrade();
        glib::spawn_future_local(async move {
            let result =
                crate::backend::run_blocking(move || crate::backend::actions::changelog(id)).await;
            let Some(row) = row_weak.upgrade() else {
                return;
            };
            match result {
                Some(Ok(text)) => {
                    row.set_subtitle("Loaded");
                    let buffer = gtk::TextBuffer::new(None);
                    buffer.set_text(&text);
                    let view = gtk::TextView::builder()
                        .buffer(&buffer)
                        .editable(false)
                        .monospace(true)
                        .top_margin(8)
                        .bottom_margin(8)
                        .left_margin(8)
                        .right_margin(8)
                        .build();
                    let scroller = gtk::ScrolledWindow::builder()
                        .child(&view)
                        .min_content_height(240)
                        .build();
                    row.add_row(&scroller);
                }
                Some(Err(e)) => row.set_subtitle(&e),
                None => row.set_subtitle("Failed to load the changelog"),
            }
        });
    });
    expander
}

fn build_rollback_row(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    pkg: &Package,
    on_changed: Rc<dyn Fn()>,
) -> adw::ActionRow {
    let source = pkg.source.unwrap_or(SourceId::Archive);
    let row = adw::ActionRow::builder()
        .title("Previous version available")
        .subtitle("Checking…")
        .build();
    let rollback_button = gtk::Button::with_label("Roll Back");
    rollback_button.set_valign(gtk::Align::Center);
    rollback_button.set_sensitive(false);
    row.add_suffix(&rollback_button);
    row.set_visible(false);

    {
        let id = pkg.id.clone();
        let row_weak = row.downgrade();
        let rollback_button_weak = rollback_button.downgrade();
        glib::spawn_future_local(async move {
            let snapshots = crate::backend::run_blocking(move || {
                crate::backend::actions::list_snapshots(source, id)
            })
            .await
            .unwrap_or_default();
            let (Some(row), Some(button)) = (row_weak.upgrade(), rollback_button_weak.upgrade())
            else {
                return;
            };
            let Some(latest) = snapshots.first() else {
                return; // stays hidden: nothing to roll back to
            };
            row.set_subtitle(&format!(
                "From {} ({} {})",
                latest
                    .created_at
                    .with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M"),
                latest.manual_install.name,
                latest.manual_install.version
            ));
            row.set_visible(true);
            button.set_sensitive(true);
        });
    }

    let id = pkg.id.clone();
    let overlay = overlay.clone();
    let stack = stack.clone();
    rollback_button.connect_clicked(move |button| {
        button.set_sensitive(false);
        let overlay = overlay.clone();
        let stack = stack.clone();
        let on_changed = on_changed.clone();
        let id = id.clone();
        glib::spawn_future_local(async move {
            let confirmed = super::confirm(
                &stack,
                "Roll back to the previous version?",
                "This replaces the currently installed version with the one from the snapshot.",
                "Roll Back",
                false,
            )
            .await;
            if !confirmed {
                return;
            }
            let outcome =
                crate::backend::run_blocking(move || crate::backend::actions::rollback(source, id))
                    .await;
            if let Some(outcome) = outcome {
                toast(&overlay, outcome.summary);
                if outcome.success {
                    on_changed();
                    super::show_list(&stack);
                }
            }
        });
    });

    row
}

fn build_actions(
    overlay: &adw::ToastOverlay,
    stack: &gtk::Stack,
    pkg: &Package,
    on_changed: Rc<dyn Fn()>,
) -> gtk::Box {
    let source = pkg.source.unwrap_or(SourceId::Apt);
    let id = pkg.id.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(6);

    if pkg.installed {
        if pkg.available_version.is_some() {
            let update_button = gtk::Button::with_label("Update");
            update_button.add_css_class("suggested-action");
            let overlay = overlay.clone();
            let stack = stack.clone();
            let on_changed = on_changed.clone();
            let id = id.clone();
            update_button.connect_clicked(move |button| {
                button.set_sensitive(false);
                let overlay = overlay.clone();
                let stack = stack.clone();
                let on_changed = on_changed.clone();
                let id = id.clone();
                glib::spawn_future_local(async move {
                    let dialog = Rc::new(crate::progress::open(&stack, "Updating"));
                    let dialog_for_sink = dialog.clone();
                    let (result, lines) = crate::backend::run_streaming(move |sink| {
                        crate::backend::actions::update_streaming(source, id, sink)
                    });
                    glib::spawn_future_local(async move {
                        while let Ok(progress) = lines.recv().await {
                            dialog_for_sink.append_line(progress.stream, &progress.line);
                        }
                    });
                    if let Some(outcome) = result.await {
                        dialog.finish(outcome.success, &outcome.summary);
                        toast(&overlay, outcome.summary);
                        if outcome.success {
                            on_changed();
                            super::show_list(&stack);
                        }
                    }
                });
            });
            row.append(&update_button);
        }

        let remove_button = gtk::Button::with_label("Remove");
        remove_button.add_css_class("destructive-action");
        {
            let overlay = overlay.clone();
            let stack = stack.clone();
            let on_changed = on_changed.clone();
            let id_for_remove = id.clone();
            remove_button.connect_clicked(move |button| {
                button.set_sensitive(false);
                let overlay = overlay.clone();
                let stack = stack.clone();
                let on_changed = on_changed.clone();
                let id = id_for_remove.clone();
                glib::spawn_future_local(async move {
                    let extra = crate::backend::run_blocking({
                        let id = id.clone();
                        move || crate::backend::actions::removal_preview(source, id)
                    })
                    .await
                    .unwrap_or_default();
                    let body = if extra.is_empty() {
                        "This cannot be undone from here.".to_string()
                    } else {
                        format!(
                            "This will also remove {} other package(s): {}",
                            extra.len(),
                            extra.join(", ")
                        )
                    };
                    if !super::confirm(&stack, "Remove this package?", &body, "Remove", true).await
                    {
                        return;
                    }
                    let dialog = Rc::new(crate::progress::open(&stack, "Removing"));
                    let dialog_for_sink = dialog.clone();
                    let (result, lines) = crate::backend::run_streaming(move |sink| {
                        crate::backend::actions::remove_streaming(source, id, sink)
                    });
                    glib::spawn_future_local(async move {
                        while let Ok(progress) = lines.recv().await {
                            dialog_for_sink.append_line(progress.stream, &progress.line);
                        }
                    });
                    if let Some(outcome) = result.await {
                        dialog.finish(outcome.success, &outcome.summary);
                        toast(&overlay, outcome.summary);
                        if outcome.success {
                            on_changed();
                            super::show_list(&stack);
                        }
                    }
                });
            });
        }
        row.append(&remove_button);

        let pin_button = gtk::Button::with_label("Pin");
        pin_button.set_valign(gtk::Align::Center);
        pin_button.set_tooltip_text(Some(
            "Hold back from bulk updates (manage pins from the Sources page)",
        ));
        let overlay = overlay.clone();
        let id_for_pin = id.clone();
        pin_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            let overlay = overlay.clone();
            let id = id_for_pin.clone();
            let button = button.clone();
            glib::spawn_future_local(async move {
                let outcome = crate::backend::run_blocking(move || {
                    crate::backend::actions::pin(source, id, None)
                })
                .await;
                if let Some(outcome) = outcome {
                    toast(&overlay, outcome.summary);
                    button.set_label("Pinned");
                } else {
                    button.set_sensitive(true);
                }
            });
        });
        row.append(&pin_button);
    } else {
        let install_button = gtk::Button::with_label("Install");
        install_button.add_css_class("suggested-action");
        let overlay = overlay.clone();
        let stack = stack.clone();
        let on_changed = on_changed.clone();
        install_button.connect_clicked(move |button| {
            button.set_sensitive(false);
            let overlay = overlay.clone();
            let stack = stack.clone();
            let on_changed = on_changed.clone();
            let id = id.clone();
            glib::spawn_future_local(async move {
                let dialog = Rc::new(crate::progress::open(&stack, "Installing"));
                let dialog_for_sink = dialog.clone();
                let (result, lines) = crate::backend::run_streaming(move |sink| {
                    crate::backend::actions::install_named_streaming(source, id, sink)
                });
                glib::spawn_future_local(async move {
                    while let Ok(progress) = lines.recv().await {
                        dialog_for_sink.append_line(progress.stream, &progress.line);
                    }
                });
                if let Some(outcome) = result.await {
                    dialog.finish(outcome.success, &outcome.summary);
                    toast(&overlay, outcome.summary);
                    if outcome.success {
                        on_changed();
                        super::show_list(&stack);
                    }
                }
            });
        });
        row.append(&install_button);
    }

    row
}
