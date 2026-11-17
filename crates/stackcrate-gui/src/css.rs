//! Small custom stylesheet layered on top of the system GTK theme: just
//! enough to turn the plain-text source labels (apt/flatpak/snap/...) into
//! small colored pills, the way most GNOME software-management apps
//! visually distinguish package origins. Everything else in the GUI
//! intentionally uses stock GTK/Adwaita widgets and styling, following the
//! system theme (light/dark/accent color) automatically.

const STYLESHEET: &str = "
.source-badge {
    padding: 1px 9px;
    border-radius: 999px;
    font-size: 0.85em;
    font-weight: 600;
}
.source-apt, .source-deb-file {
    background-color: alpha(@accent_bg_color, 0.18);
    color: @accent_color;
}
.source-archive, .source-appimage {
    background-color: alpha(#9141ac, 0.18);
    color: #9141ac;
}
.source-flatpak {
    background-color: alpha(#1c71d8, 0.18);
    color: #1c71d8;
}
.source-snap {
    background-color: alpha(#e5a50a, 0.25);
    color: #9c6e03;
}

.source-filter-toggle {
    border-radius: 999px;
    font-size: 0.85em;
    font-weight: 600;
    padding-left: 10px;
    padding-right: 10px;
    min-height: 0;
}
.source-filter-toggle:checked.source-apt, .source-filter-toggle:checked.source-deb-file {
    background-color: alpha(@accent_bg_color, 0.28);
    color: @accent_color;
}
.source-filter-toggle:checked.source-archive, .source-filter-toggle:checked.source-appimage {
    background-color: alpha(#9141ac, 0.28);
    color: #9141ac;
}
.source-filter-toggle:checked.source-flatpak {
    background-color: alpha(#1c71d8, 0.28);
    color: #1c71d8;
}
.source-filter-toggle:checked.source-snap {
    background-color: alpha(#e5a50a, 0.32);
    color: #9c6e03;
}
";

pub fn install() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(STYLESHEET);
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// CSS class for a source badge pill, matching the rules above.
pub fn source_class(source: stackcrate_core::types::SourceId) -> &'static str {
    use stackcrate_core::types::SourceId;
    match source {
        SourceId::Apt => "source-apt",
        SourceId::DebFile => "source-deb-file",
        SourceId::Archive => "source-archive",
        SourceId::AppImage => "source-appimage",
        SourceId::Flatpak => "source-flatpak",
        SourceId::Snap => "source-snap",
    }
}
