pub mod installed;
pub mod search;
pub mod updates;

use adw::prelude::*;

/// Common page chrome: a toolbar row and a scrollable list. Toasts are shown
/// through one overlay shared by the whole window (see `main.rs`), not a
/// separate one per page, so a window-level action like "Install File..."
/// has somewhere to report results regardless of which page is active.
pub struct PageScaffold {
    pub root: gtk::Box,
    pub toolbar: gtk::Box,
    pub list: gtk::ListBox,
}

pub fn scaffold() -> PageScaffold {
    let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    toolbar.set_margin_top(12);
    toolbar.set_margin_bottom(6);
    toolbar.set_margin_start(12);
    toolbar.set_margin_end(12);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_bottom(12);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&list));
    scroller.set_vexpand(true);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&toolbar);
    root.append(&scroller);

    PageScaffold {
        root,
        toolbar,
        list,
    }
}

pub fn toast(overlay: &adw::ToastOverlay, message: impl AsRef<str>) {
    overlay.add_toast(adw::Toast::new(message.as_ref()));
}

pub fn source_badge_text(source: pallet_core::types::SourceId) -> &'static str {
    match source {
        pallet_core::types::SourceId::Apt => "apt",
        pallet_core::types::SourceId::DebFile => "deb",
        pallet_core::types::SourceId::Archive => "archive",
        pallet_core::types::SourceId::AppImage => "appimage",
        pallet_core::types::SourceId::Flatpak => "flatpak",
        pallet_core::types::SourceId::Snap => "snap",
    }
}

pub fn clear_list(list: &gtk::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}
