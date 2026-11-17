mod backend;
mod pages;
mod settings;

use adw::prelude::*;
use gtk::gio;
use gtk::glib;

const APP_ID: &str = "org.pallet.Pallet";

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "pallet_gui=info".into()),
        )
        .with_target(false)
        .init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let _ = pallet_core::config::ensure_dirs();

    let toast_overlay = adw::ToastOverlay::new();

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &pages::search::build(&toast_overlay),
        Some("search"),
        "Search",
        "system-search-symbolic",
    );
    stack.add_titled_with_icon(
        &pages::installed::build(&toast_overlay),
        Some("installed"),
        "Installed",
        "object-select-symbolic",
    );
    stack.add_titled_with_icon(
        &pages::updates::build(&toast_overlay),
        Some("updates"),
        "Updates",
        "software-update-available-symbolic",
    );
    toast_overlay.set_child(Some(&stack));

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let install_file_button = gtk::Button::from_icon_name("document-open-symbolic");
    install_file_button.set_tooltip_text(Some("Install a local .deb, archive, or AppImage file"));

    let settings_button = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some("Settings"));

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    header.pack_start(&install_file_button);
    header.pack_end(&settings_button);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&toast_overlay));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Pallet")
        .default_width(880)
        .default_height(640)
        .content(&toolbar_view)
        .build();

    {
        let window_weak = window.downgrade();
        settings_button.connect_clicked(move |_| {
            if let Some(window) = window_weak.upgrade() {
                settings::open(&window);
            }
        });
    }

    {
        let window_weak = window.downgrade();
        let toast_overlay = toast_overlay.clone();
        let stack = stack.clone();
        install_file_button.connect_clicked(move |_| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let toast_overlay = toast_overlay.clone();
            let stack = stack.clone();
            glib::spawn_future_local(async move {
                prompt_install_file(&window, &toast_overlay, &stack).await;
            });
        });
    }

    window.present();
}

async fn prompt_install_file(
    window: &adw::ApplicationWindow,
    overlay: &adw::ToastOverlay,
    stack: &adw::ViewStack,
) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Installable files (.deb, archive, AppImage)"));
    for pattern in [
        "*.deb",
        "*.AppImage",
        "*.appimage",
        "*.tar.gz",
        "*.tar.xz",
        "*.tar.bz2",
        "*.tgz",
        "*.txz",
        "*.zip",
    ] {
        filter.add_pattern(pattern);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);

    let dialog = gtk::FileDialog::builder()
        .title("Choose a file to install")
        .filters(&filters)
        .build();

    let file = match dialog.open_future(Some(window)).await {
        Ok(file) => file,
        Err(_) => return, // user cancelled; nothing to report
    };
    let Some(path) = file.path() else {
        pages::toast(overlay, "Could not resolve the chosen file's path");
        return;
    };

    pages::toast(overlay, format!("Installing {}...", path.display()));
    let outcome =
        crate::backend::run_blocking(move || crate::backend::actions::install_file(path)).await;
    match outcome {
        Some(outcome) => {
            pages::toast(overlay, outcome.summary);
            if outcome.success {
                stack.set_visible_child_name("installed");
            }
        }
        None => pages::toast(overlay, "Install failed unexpectedly"),
    }
}
