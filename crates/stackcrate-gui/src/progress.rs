//! A small modal dialog showing live, line-by-line output from a
//! long-running operation (install/remove/update/autoremove), fed by the
//! `ProgressLine` channel `backend::run_streaming` produces. Previously
//! every long operation was a plain "Installing..." button with no visible
//! feedback until it finished; this gives the same real-time view the CLI's
//! `--verbose` log already had access to.

use adw::prelude::*;
use gtk::glib;
use stackcrate_core::types::StreamKind;

pub struct ProgressDialog {
    dialog: adw::Dialog,
    buffer: gtk::TextBuffer,
    text_view: gtk::TextView,
    status_label: gtk::Label,
    close_button: gtk::Button,
}

pub fn open(parent: &impl IsA<gtk::Widget>, title: &str) -> ProgressDialog {
    let buffer = gtk::TextBuffer::new(None);
    let text_view = gtk::TextView::builder()
        .buffer(&buffer)
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(10)
        .right_margin(10)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&text_view)
        .vexpand(true)
        .build();

    let status_label = gtk::Label::new(Some("Running…"));
    status_label.add_css_class("dim-label");
    status_label.set_halign(gtk::Align::Start);
    status_label.set_hexpand(true);
    status_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

    let close_button = gtk::Button::with_label("Close");
    close_button.set_sensitive(false);
    close_button.add_css_class("suggested-action");

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    header.set_title_widget(Some(&gtk::Label::new(Some(title))));

    let bottom_bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bottom_bar.set_margin_top(6);
    bottom_bar.set_margin_bottom(6);
    bottom_bar.set_margin_start(12);
    bottom_bar.set_margin_end(12);
    bottom_bar.append(&status_label);
    bottom_bar.append(&close_button);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.add_bottom_bar(&bottom_bar);
    toolbar_view.set_content(Some(&scroller));

    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(640)
        .content_height(420)
        .can_close(false) // only the Close button (enabled once finished) dismisses it
        .child(&toolbar_view)
        .build();

    {
        let dialog_weak = dialog.downgrade();
        close_button.connect_clicked(move |_| {
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.close();
            }
        });
    }

    dialog.present(Some(parent));

    ProgressDialog {
        dialog,
        buffer,
        text_view,
        status_label,
        close_button,
    }
}

impl ProgressDialog {
    pub fn append_line(&self, stream: StreamKind, line: &str) {
        let mut end = self.buffer.end_iter();
        if stream == StreamKind::Stderr {
            self.buffer.insert_markup(
                &mut end,
                &format!(
                    "<span alpha=\"75%\">{}</span>\n",
                    glib::markup_escape_text(line)
                ),
            );
        } else {
            self.buffer.insert(&mut end, line);
            self.buffer.insert(&mut end, "\n");
        }
        let insert_mark = self.buffer.get_insert();
        self.text_view
            .scroll_to_mark(&insert_mark, 0.0, false, 0.0, 0.0);
    }

    pub fn finish(&self, success: bool, summary: &str) {
        self.status_label.set_text(summary);
        self.status_label.remove_css_class("dim-label");
        self.status_label
            .add_css_class(if success { "success" } else { "error" });
        self.close_button.set_sensitive(true);
        self.dialog.set_can_close(true);
    }
}
