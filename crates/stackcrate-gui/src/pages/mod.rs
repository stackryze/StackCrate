pub mod detail;
pub mod filters;
pub mod history;
pub mod installed;
pub mod search;
pub mod sources;
pub mod updates;

use adw::prelude::*;

/// Common page chrome: a toolbar row and a scrollable list, held inside a
/// plain `gtk::Stack` with two children ("list" and, once a row has been
/// clicked, "detail"). A `gtk::Stack` rather than a bare `gtk::Box` so a
/// page can show a package's detail view in place without needing a second
/// window/dialog or restructuring the window's outer chrome (header,
/// view switcher, global actions), which stays exactly as before.
pub struct PageScaffold {
    pub root: gtk::Stack,
    /// The "list" child's own container, exposed so a page can insert extra
    /// chrome (e.g. an `adw::Banner`) between the toolbar and the list.
    pub list_page: gtk::Box,
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

    let list_page = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list_page.append(&toolbar);
    list_page.append(&scroller);

    let root = gtk::Stack::new();
    root.set_transition_type(gtk::StackTransitionType::SlideLeftRight);
    root.add_named(&list_page, Some("list"));

    PageScaffold {
        root,
        list_page,
        toolbar,
        list,
    }
}

/// Replaces whatever detail view is currently showing (if any) with
/// `widget` and switches to it. The caller builds a fresh widget each time
/// rather than this trying to reuse/update one in place, since a different
/// package's detail is a different page in all but name.
pub fn show_detail(stack: &gtk::Stack, widget: &impl IsA<gtk::Widget>) {
    if let Some(old) = stack.child_by_name("detail") {
        stack.remove(&old);
    }
    stack.add_named(widget, Some("detail"));
    stack.set_visible_child_name("detail");
}

pub fn show_list(stack: &gtk::Stack) {
    stack.set_visible_child_name("list");
}

pub fn toast(overlay: &adw::ToastOverlay, message: impl AsRef<str>) {
    overlay.add_toast(adw::Toast::new(message.as_ref()));
}

pub fn source_badge_text(source: stackcrate_core::types::SourceId) -> &'static str {
    match source {
        stackcrate_core::types::SourceId::Apt => "apt",
        stackcrate_core::types::SourceId::DebFile => "deb",
        stackcrate_core::types::SourceId::Archive => "archive",
        stackcrate_core::types::SourceId::AppImage => "appimage",
        stackcrate_core::types::SourceId::Flatpak => "flatpak",
        stackcrate_core::types::SourceId::Snap => "snap",
    }
}

/// A small colored pill (see `css.rs`) instead of plain dimmed text, so
/// source origin reads as a label at a glance rather than trailing text.
pub fn source_badge(source: stackcrate_core::types::SourceId) -> gtk::Label {
    let badge = gtk::Label::new(Some(source_badge_text(source)));
    badge.add_css_class("source-badge");
    badge.add_css_class(crate::css::source_class(source));
    badge.set_valign(gtk::Align::Center);
    badge
}

pub fn clear_list(list: &gtk::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        list.remove(&row);
    }
}

/// A friendlier placeholder row than a bare text label: a leading icon plus
/// a title/subtitle pair, for empty results, "nothing installed yet", and
/// similar not-really-an-error states.
pub fn placeholder_row(icon_name: &str, title: &str, subtitle: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(title).build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    let image = gtk::Image::from_icon_name(icon_name);
    image.set_pixel_size(28);
    image.add_css_class("dim-label");
    row.add_prefix(&image);
    row
}

/// A row with a spinner, shown while a list is (re)loading.
pub fn loading_row(label: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder().title(label).build();
    // A plain `gtk::Spinner`, not `adw::Spinner` (libadwaita 1.6+ only --
    // this needs to keep working against Ubuntu 24.04 LTS's packaged 1.5),
    // so `.start()` is required (unlike `AdwSpinner`, which just spins
    // whenever visible with no explicit start/stop state).
    let spinner = gtk::Spinner::new();
    spinner.start();
    spinner.set_width_request(20);
    spinner.set_height_request(20);
    spinner.set_valign(gtk::Align::Center);
    row.add_prefix(&spinner);
    row
}

/// A Cancel/Confirm alert dialog; returns whether the user confirmed.
/// `destructive` styles the confirm button for a destructive action (red),
/// otherwise it's styled as the suggested (primary) action.
pub async fn confirm(
    parent: &impl IsA<gtk::Widget>,
    heading: &str,
    body: &str,
    confirm_label: &str,
    destructive: bool,
) -> bool {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("confirm", confirm_label);
    dialog.set_response_appearance(
        "confirm",
        if destructive {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog.choose_future(Some(parent)).await == "confirm"
}
