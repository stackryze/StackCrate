//! Small, reusable filter/sort controls shared by Search, Installed, and
//! Updates: a row of per-source toggle buttons, a local text filter, and a
//! sort dropdown. Each page owns its own copy of its full (unfiltered)
//! dataset and re-renders from it whenever a control here changes, rather
//! than re-querying anything -- filtering/sorting what's already loaded is
//! instant and needs no network/subprocess round trip.

use adw::prelude::*;
use stackcrate_core::types::SourceId;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// A row of toggle buttons, one per source in `available`, all active
/// (pressed = included) by default. Returns the row plus the per-source
/// buttons so the caller can wire `connect_toggled` itself -- this doesn't
/// take an `on_changed` callback at construction time because callers
/// generally want to wire it to a `render` closure that's defined *after*
/// all the filter widgets exist (so it can read all of them), which a
/// constructor-time callback can't do without an awkward two-phase dance.
#[derive(Clone)]
pub struct SourceFilter {
    pub widget: gtk::Box,
    pub buttons: Vec<(SourceId, gtk::ToggleButton)>,
    active: Rc<RefCell<HashSet<SourceId>>>,
}

impl SourceFilter {
    pub fn is_active(&self, source: SourceId) -> bool {
        self.active.borrow().contains(&source)
    }

    /// Connects every button's toggle signal to call `on_changed` after
    /// updating the active set. Call once, after `on_changed` is defined.
    pub fn connect_changed(&self, on_changed: Rc<dyn Fn()>) {
        for (source, button) in &self.buttons {
            let source = *source;
            let active = self.active.clone();
            let on_changed = on_changed.clone();
            button.connect_toggled(move |btn| {
                if btn.is_active() {
                    active.borrow_mut().insert(source);
                } else {
                    active.borrow_mut().remove(&source);
                }
                on_changed();
            });
        }
    }
}

pub fn source_filter(available: &[SourceId]) -> SourceFilter {
    let active: Rc<RefCell<HashSet<SourceId>>> =
        Rc::new(RefCell::new(available.iter().copied().collect()));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.set_valign(gtk::Align::Center);
    let mut buttons = Vec::new();
    for source in available {
        let source = *source;
        let button = gtk::ToggleButton::builder()
            .label(super::source_badge_text(source))
            .active(true)
            .tooltip_text(format!(
                "Show/hide {} results",
                super::source_badge_text(source)
            ))
            .build();
        button.add_css_class("source-filter-toggle");
        button.add_css_class(crate::css::source_class(source));
        row.append(&button);
        buttons.push((source, button));
    }
    SourceFilter {
        widget: row,
        buttons,
        active,
    }
}

/// A `SearchEntry` used as a local (client-side) text filter rather than a
/// remote query -- distinct from the Search page's own query entry, which
/// *does* trigger a network/subprocess call. Wire `connect_search_changed`
/// yourself once your `render` closure exists.
pub fn text_filter(placeholder: &str) -> gtk::SearchEntry {
    let entry = gtk::SearchEntry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_hexpand(true);
    entry
}

pub fn matches_text(filter: &str, haystacks: &[&str]) -> bool {
    if filter.trim().is_empty() {
        return true;
    }
    let filter = filter.to_ascii_lowercase();
    haystacks
        .iter()
        .any(|h| h.to_ascii_lowercase().contains(&filter))
}

/// A sort dropdown; `options` are display labels, selection is read back as
/// a plain index for the caller to interpret with its own enum. Wire
/// `connect_selected_notify` yourself once your `render` closure exists.
pub fn sort_dropdown(options: &[&str]) -> gtk::DropDown {
    let model = gtk::StringList::new(options);
    gtk::DropDown::builder().model(&model).build()
}

/// A second toolbar-like row for filter controls, inserted right after the
/// main toolbar (see `PageScaffold::list_page`/`insert_child_after`).
pub fn filter_row() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(2);
    row.set_margin_bottom(6);
    row.set_margin_start(12);
    row.set_margin_end(12);
    row
}
