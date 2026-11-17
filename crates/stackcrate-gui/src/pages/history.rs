use super::{clear_list, loading_row, placeholder_row, scaffold, source_badge};
use adw::prelude::*;
use gtk::glib;
use stackcrate_core::registry::HistoryEntry;

pub fn build() -> gtk::Stack {
    let super::PageScaffold {
        root,
        toolbar,
        list,
        ..
    } = scaffold();

    let label = gtk::Label::new(Some("Recent install/remove/update activity"));
    label.set_hexpand(true);
    label.set_halign(gtk::Align::Start);
    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Refresh"));
    toolbar.append(&label);
    toolbar.append(&refresh_button);

    let do_refresh = {
        let list = list.clone();
        move || {
            clear_list(&list);
            list.append(&loading_row("Loading history…"));
            let list = list.clone();
            glib::spawn_future_local(async move {
                let entries =
                    crate::backend::run_blocking(|| crate::backend::actions::recent_history(100))
                        .await;
                clear_list(&list);
                match entries {
                    Some(entries) if !entries.is_empty() => {
                        for entry in entries {
                            list.append(&build_row(&entry));
                        }
                    }
                    _ => list.append(&placeholder_row(
                        "document-open-recent-symbolic",
                        "No history yet",
                        "Installs, removals, and updates will show up here.",
                    )),
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

fn build_row(entry: &HistoryEntry) -> adw::ActionRow {
    let when = entry
        .timestamp
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M");
    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&entry.package).to_string())
        .subtitle(format!(
            "{when} · {}",
            glib::markup_escape_text(&entry.detail)
        ))
        .build();

    let source: stackcrate_core::types::SourceId = entry
        .source
        .parse()
        .unwrap_or(stackcrate_core::types::SourceId::Apt);
    row.add_prefix(&source_badge(source));

    let action_label = gtk::Label::new(Some(&entry.action));
    action_label.add_css_class("dim-label");
    row.add_suffix(&action_label);

    let mark = gtk::Image::from_icon_name(if entry.success {
        "emblem-ok-symbolic"
    } else {
        "dialog-error-symbolic"
    });
    if !entry.success {
        mark.add_css_class("error");
    } else {
        mark.add_css_class("success");
    }
    row.add_suffix(&mark);

    row
}
