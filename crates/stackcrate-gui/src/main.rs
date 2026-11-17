mod backend;
mod css;
mod pages;
mod progress;
mod settings;
mod state;

use adw::prelude::*;
use gtk::gio;
use gtk::glib;

const APP_ID: &str = "org.stackcrate.StackCrate";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "stackcrate_gui=info".into()),
        )
        .with_target(false)
        .init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| css::install());
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &adw::Application) {
    let _ = stackcrate_core::config::ensure_dirs();
    let window_state = state::WindowState::load();

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
    stack.add_titled_with_icon(
        &pages::history::build(),
        Some("history"),
        "History",
        "document-open-recent-symbolic",
    );
    stack.add_titled_with_icon(
        &pages::sources::build(&toast_overlay),
        Some("sources"),
        "Sources",
        "preferences-other-symbolic",
    );
    toast_overlay.set_child(Some(&stack));

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let install_file_button = gtk::Button::from_icon_name("document-open-symbolic");
    install_file_button.set_tooltip_text(Some("Install a local .deb, archive, or AppImage file"));
    install_file_button
        .update_property(&[gtk::accessible::Property::Label("Install a local file")]);

    let menu_button = gtk::MenuButton::new();
    menu_button.set_icon_name("open-menu-symbolic");
    menu_button.set_tooltip_text(Some("Main Menu"));
    let primary_menu = gio::Menu::new();
    primary_menu.append(Some("_Keyboard Shortcuts"), Some("win.show-shortcuts"));
    primary_menu.append(Some("_About StackCrate"), Some("win.show-about"));
    menu_button.set_menu_model(Some(&primary_menu));

    let settings_button = gtk::Button::from_icon_name("emblem-system-symbolic");
    settings_button.set_tooltip_text(Some("Settings"));
    settings_button.update_property(&[gtk::accessible::Property::Label("Settings")]);

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(&switcher));
    header.pack_start(&install_file_button);
    header.pack_end(&menu_button);
    header.pack_end(&settings_button);

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&toast_overlay));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("StackCrate")
        .icon_name(APP_ID)
        .default_width(window_state.width)
        .default_height(window_state.height)
        .maximized(window_state.maximized)
        .content(&toolbar_view)
        .build();

    // ---- window actions: settings, shortcuts, about ----------------------
    let action_settings = gio::SimpleAction::new("show-settings", None);
    {
        let window_weak = window.downgrade();
        let toast_overlay = toast_overlay.clone();
        action_settings.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                settings::open(&window, &toast_overlay);
            }
        });
    }
    window.add_action(&action_settings);
    settings_button.connect_clicked(glib::clone!(
        #[weak]
        window,
        #[strong]
        toast_overlay,
        move |_| {
            settings::open(&window, &toast_overlay);
        }
    ));

    let action_shortcuts = gio::SimpleAction::new("show-shortcuts", None);
    {
        let window_weak = window.downgrade();
        action_shortcuts.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                show_shortcuts_dialog(&window);
            }
        });
    }
    window.add_action(&action_shortcuts);

    let action_about = gio::SimpleAction::new("show-about", None);
    {
        let window_weak = window.downgrade();
        action_about.connect_activate(move |_, _| {
            if let Some(window) = window_weak.upgrade() {
                show_about_dialog(&window);
            }
        });
    }
    window.add_action(&action_about);

    app.set_accels_for_action("win.show-settings", &["<primary>comma"]);
    app.set_accels_for_action("win.show-shortcuts", &["<primary>question"]);
    app.set_accels_for_action("app.quit", &["<primary>q"]);
    app.set_accels_for_action("window.close", &["<primary>w"]);

    let action_quit = gio::SimpleAction::new("quit", None);
    {
        let app_weak = app.downgrade();
        action_quit.connect_activate(move |_, _| {
            if let Some(app) = app_weak.upgrade() {
                app.quit();
            }
        });
    }
    app.add_action(&action_quit);

    // ---- install-file button (file picker) --------------------------------
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

    // ---- drag-and-drop a file onto the window to install it --------------
    {
        let drop_target =
            gtk::DropTarget::new(gio::File::static_type(), gtk::gdk::DragAction::COPY);
        drop_target.set_types(&[gtk::gdk::FileList::static_type(), gio::File::static_type()]);
        let overlay = toast_overlay.clone();
        let stack = stack.clone();
        drop_target.connect_drop(move |_, value, _, _| {
            let path = value
                .get::<gtk::gdk::FileList>()
                .ok()
                .and_then(|list| list.files().into_iter().next())
                .or_else(|| value.get::<gio::File>().ok())
                .and_then(|file| file.path());
            let Some(path) = path else {
                return false;
            };
            let overlay = overlay.clone();
            let stack = stack.clone();
            glib::spawn_future_local(async move {
                install_file_at_path(&overlay, &stack, path).await;
            });
            true
        });
        toolbar_view.add_controller(drop_target);
    }

    // ---- persist window size/maximized state on close ----------------------
    {
        let window_weak = window.downgrade();
        window.connect_close_request(move |_| {
            if let Some(window) = window_weak.upgrade() {
                let maximized = window.is_maximized();
                let (width, height) = if maximized {
                    (window_state.width, window_state.height)
                } else {
                    (window.width(), window.height())
                };
                state::WindowState {
                    width,
                    height,
                    maximized,
                }
                .save();
            }
            glib::Propagation::Proceed
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
    install_file_at_path(overlay, stack, path).await;
}

async fn install_file_at_path(
    overlay: &adw::ToastOverlay,
    stack: &adw::ViewStack,
    path: std::path::PathBuf,
) {
    let dialog = std::rc::Rc::new(crate::progress::open(stack, "Installing"));
    let dialog_for_sink = dialog.clone();
    let (result, lines) = crate::backend::run_streaming(move |sink| {
        crate::backend::actions::install_file_streaming(path, sink)
    });
    glib::spawn_future_local(async move {
        while let Ok(progress) = lines.recv().await {
            dialog_for_sink.append_line(progress.stream, &progress.line);
        }
    });
    match result.await {
        Some(outcome) => {
            dialog.finish(outcome.success, &outcome.summary);
            pages::toast(overlay, outcome.summary);
            if outcome.success {
                stack.set_visible_child_name("installed");
            }
        }
        None => {
            dialog.finish(false, "Install failed unexpectedly");
            pages::toast(overlay, "Install failed unexpectedly");
        }
    }
}

fn show_about_dialog(window: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::builder()
        .application_name("StackCrate")
        .application_icon(APP_ID)
        .version(APP_VERSION)
        .developer_name("StackCrate contributors")
        .license_type(gtk::License::MitX11)
        .website("https://github.com/stackryze/StackCrate")
        .issue_url("https://github.com/stackryze/StackCrate/issues")
        .comments(
            "Unifies apt, local .deb files, archives, AppImages, Flatpak, and Snap behind one \
             CLI, background monitor, and app.",
        )
        .build();
    about.add_credit_section(
        Some("Reference"),
        &["Adapter command semantics informed by OmniPkg (github.com/grosserknallkopf/OmniPkg)"],
    );
    about.present(Some(window));
}

fn show_shortcuts_dialog(window: &adw::ApplicationWindow) {
    let dialog = adw::Dialog::builder()
        .title("Keyboard Shortcuts")
        .content_width(420)
        .content_height(320)
        .build();

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    header.set_title_widget(Some(&gtk::Label::new(Some("Keyboard Shortcuts"))));

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");
    list.set_margin_start(12);
    list.set_margin_end(12);
    list.set_margin_top(6);
    list.set_margin_bottom(12);
    for (accelerator, action) in [
        ("<primary>comma", "Open Settings"),
        ("<primary>question", "Keyboard Shortcuts (this window)"),
        ("<primary>w", "Close Window"),
        ("<primary>q", "Quit"),
    ] {
        let row = adw::ActionRow::builder().title(action).build();
        let shortcut_label = gtk::ShortcutLabel::new(accelerator);
        shortcut_label.set_valign(gtk::Align::Center);
        row.add_suffix(&shortcut_label);
        list.append(&row);
    }

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&list));
    dialog.set_child(Some(&toolbar_view));
    dialog.present(Some(window));
}
