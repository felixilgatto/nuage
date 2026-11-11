//! `nuage-gtk`: the Nuage window.
//!
//! A client of `nuaged`, which it starts if needed, and of the tray agent
//! (`nuage-tray`), which it starts too: the agent stays running (icon,
//! notifications, updates) and this program ends with its window.
//!
//! Launched by the user, it opens the setup (no account yet) or the main
//! window. The agent runs it with `--action NAME [--target VALUE]` for what
//! needs a window (preferences, signing in again, a question before
//! quitting); a running instance gets the request. `--background` (the
//! login entry of older versions) only starts the agent.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use nuage_core::{AccountId, Command, Event};
use nuage_ui::{Backend, BackendEvent, Ui};

/// Where translations are installed (overridable for development).
const DEFAULT_LOCALE_DIR: &str = "/app/share/locale";

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
    /// The start is done (daemon reached or not, state loaded).
    static READY: Cell<bool> = const { Cell::new(false) };
}

fn current_ui() -> Option<Rc<Ui>> {
    UI.with(|u| u.borrow().clone())
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NUAGE_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let locale_dir =
        std::env::var("NUAGE_LOCALE_DIR").unwrap_or_else(|_| DEFAULT_LOCALE_DIR.to_owned());
    nuage_ui::init_i18n(&locale_dir);

    let app = adw::Application::builder()
        .application_id(nuage_core::APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(startup);
    app.connect_command_line(command_line);
    app.run()
}

/// What a launch asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Request {
    /// The window (or the setup, without accounts).
    Window,
    /// Only make sure the daemon and the agent run.
    Background,
    /// An application action, with its string target if any.
    Action(String, Option<String>),
}

impl Request {
    fn parse(args: &[String]) -> Self {
        let value = |name: &str| {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        if let Some(action) = value("--action") {
            Self::Action(action, value("--target"))
        } else if args.iter().any(|a| a == "--background") {
            Self::Background
        } else {
            Self::Window
        }
    }
}

/// Every launch, the first one included. The request is served once the
/// start is done; the application is held meanwhile.
fn command_line(app: &adw::Application, cmdline: &gio::ApplicationCommandLine) -> glib::ExitCode {
    let args: Vec<String> = cmdline
        .arguments()
        .iter()
        .skip(1)
        .filter_map(|a| a.to_str().map(str::to_owned))
        .collect();
    let request = Request::parse(&args);
    let hold = app.hold();
    let app = app.clone();
    glib::spawn_future_local(async move {
        let _hold = hold;
        while !READY.with(Cell::get) {
            glib::timeout_future(Duration::from_millis(50)).await;
        }
        let Some(ui) = current_ui() else { return };
        match request {
            Request::Background => {}
            Request::Window => {
                if ui.accounts().is_empty() {
                    nuage_ui::onboarding::start(&ui);
                } else {
                    ui.present();
                }
            }
            // Held until done: they may end without a window.
            Request::Action(name, _) if name == "quit-all" => quit(&app, &ui).await,
            Request::Action(name, _) if name == "restart" => restart(&app, &ui).await,
            Request::Action(name, target) => {
                let target = target.map(|t| t.to_variant());
                if app.lookup_action(&name).is_some() {
                    app.activate_action(&name, target.as_ref());
                } else {
                    tracing::warn!(name, "unknown action");
                    ui.present();
                }
            }
        }
    });
    glib::ExitCode::SUCCESS
}

/// The icons of a source build (`data/icons`); installed icons are found
/// in the system icon theme.
fn source_icon_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons"));
    dir.is_dir().then_some(dir)
}

fn startup(app: &adw::Application) {
    if let (Some(dir), Some(display)) = (source_icon_dir(), gtk::gdk::Display::default()) {
        gtk::IconTheme::for_display(&display).add_search_path(dir);
    }
    gtk::Window::set_default_icon_name(nuage_core::APP_ID);

    let (backend, events) = match Backend::start() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "cannot start");
            READY.with(|r| r.set(true));
            return;
        }
    };
    // The windows hide when closed (they keep their state while the
    // program runs): it ends when the last visible one closes.
    app.connect_window_added(|app, window| {
        let app = app.downgrade();
        window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                glib::idle_add_local_once(move || {
                    if !app
                        .windows()
                        .iter()
                        .any(gtk::prelude::WidgetExt::is_visible)
                    {
                        app.quit();
                    }
                });
            }
            glib::Propagation::Proceed
        });
    });
    let ui = Ui::new(app, backend);
    UI.with(|u| *u.borrow_mut() = Some(Rc::clone(&ui)));
    add_actions(app, &ui);

    // Start: daemon, agent, state.
    {
        let ui = Rc::clone(&ui);
        glib::spawn_future_local(async move {
            if let Err(e) = ui.backend.ensure_daemon().await {
                ui.show_disconnected(&e);
                ui.present();
            } else {
                ui.reload().await;
            }
            if let Err(e) = nuage_tray::agent::start() {
                tracing::warn!(error = %e, "cannot start the tray agent");
            }
            READY.with(|r| r.set(true));
        });
    }

    // Events from the daemon. A crashed daemon is started again by the
    // agent.
    {
        let ui = Rc::clone(&ui);
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    BackendEvent::Connected => ui.reload().await,
                    BackendEvent::Disconnected(message) => ui.show_disconnected(&message),
                    BackendEvent::Event(Event::StatusChanged { status }) => ui.show_status(status),
                    BackendEvent::Event(_) => {}
                }
            }
        });
    }
}

fn open_folder(path: &std::path::Path) {
    if let Some(ui) = current_ui() {
        nuage_ui::open_folder(&ui.window, path);
    }
}

fn add_actions(app: &adw::Application, ui: &Rc<Ui>) {
    let action = |name: &str, f: Box<dyn Fn()>| {
        let a = gio::SimpleAction::new(name, None);
        a.connect_activate(move |_, _| f());
        app.add_action(&a);
    };
    {
        let ui = Rc::clone(ui);
        action(
            "add-account",
            Box::new(move || nuage_ui::onboarding::start(&ui)),
        );
    }
    {
        let ui = Rc::clone(ui);
        action("about", Box::new(move || nuage_ui::show_about(&ui.window)));
    }
    {
        let ui = Rc::clone(ui);
        action(
            "preferences",
            Box::new(move || {
                ui.present();
                nuage_ui::preferences::show(&ui, &ui.window);
            }),
        );
    }
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    {
        let ui = Rc::clone(ui);
        action("open-window", Box::new(move || ui.present()));
    }
    // Notification buttons, with an account ID.
    let reconnect = gio::SimpleAction::new("reconnect", Some(glib::VariantTy::STRING));
    {
        let ui = Rc::clone(ui);
        reconnect.connect_activate(move |_, param| {
            let Some(id) = param.and_then(|p| p.get::<String>()) else {
                return;
            };
            let ui = Rc::clone(&ui);
            ui.present();
            glib::spawn_future_local(async move {
                let window = ui.window.clone();
                nuage_ui::reconnect::reconnect(&ui, &AccountId::new(id), &window).await;
            });
        });
    }
    app.add_action(&reconnect);
    let account_preferences =
        gio::SimpleAction::new("account-preferences", Some(glib::VariantTy::STRING));
    {
        let ui = Rc::clone(ui);
        account_preferences.connect_activate(move |_, param| {
            let Some(id) = param.and_then(|p| p.get::<String>()) else {
                return;
            };
            ui.present();
            nuage_ui::preferences::show_account(&ui, &ui.window, &AccountId::new(id));
        });
    }
    app.add_action(&account_preferences);
    for (name, command) in [
        ("refresh", Command::Refresh { account: None }),
        ("pause-all", Command::Pause { account: None }),
        ("resume-all", Command::Resume { account: None }),
    ] {
        let ui = Rc::clone(ui);
        action(
            name,
            Box::new(move || {
                let (ui, command) = (Rc::clone(&ui), command.clone());
                glib::spawn_future_local(async move {
                    if let Err(e) = ui.backend.call(command).await {
                        ui.toast(&e);
                        ui.present();
                    }
                });
            }),
        );
    }
    {
        let ui = Rc::clone(ui);
        let app2 = app.clone();
        action(
            "quit-all",
            Box::new(move || {
                let (ui, app) = (Rc::clone(&ui), app2.clone());
                glib::spawn_future_local(async move { quit(&app, &ui).await });
            }),
        );
    }
    {
        let ui = Rc::clone(ui);
        let app2 = app.clone();
        action(
            "restart",
            Box::new(move || {
                let (ui, app) = (Rc::clone(&ui), app2.clone());
                glib::spawn_future_local(async move { restart(&app, &ui).await });
            }),
        );
    }
    {
        let ui = Rc::clone(ui);
        action("install-update", Box::new(move || ui.install_update()));
    }
    // Updates of Nuage itself: offer to install, then to restart.
    {
        let ui = Rc::clone(ui);
        let updates = ui.backend.watch_updates();
        glib::spawn_future_local(async move {
            while let Ok(event) = updates.recv().await {
                tracing::info!(?event, "update of Nuage");
                ui.update_event(event);
            }
        });
    }
    // Notification button: open a folder.
    let open = gio::SimpleAction::new("open-folder", Some(glib::VariantTy::STRING));
    open.connect_activate(|_, param| {
        if let Some(path) = param.and_then(|p| p.get::<String>()) {
            open_folder(&PathBuf::from(path));
        }
    });
    app.add_action(&open);
}

/// "Quit": stops the application, the agent and the daemon, following the
/// "quit with pending uploads" setting.
async fn quit(app: &adw::Application, ui: &Rc<Ui>) {
    if stop_daemon(ui).await {
        nuage_tray::agent::stop();
        app.quit();
    }
}

/// "Restart" after an update: quits, and the newest version starts (with
/// its own agent).
async fn restart(app: &adw::Application, ui: &Rc<Ui>) {
    if !stop_daemon(ui).await {
        return;
    }
    nuage_tray::agent::stop();
    match ui.backend.start_latest_version().await {
        Ok(()) => app.quit(),
        Err(e) => {
            tracing::warn!(error = %e, "cannot start the new version");
            ui.present();
            ui.toast(
                &gettext("Nuage could not restart: {}. Quit it and open it again.")
                    .replace("{}", &e),
            );
        }
    }
}

/// Stops the daemon, asking first if uploads are pending ("ask" setting).
/// Whether the application may quit.
async fn stop_daemon(ui: &Rc<Ui>) -> bool {
    match ui.backend.call(Command::DaemonStop { now: false }).await {
        Ok(_) => true,
        // Setting "ask": uploads are pending.
        Err(message) => {
            if !ui.backend.daemon_running().await {
                return true;
            }
            ui.present();
            let dialog = adw::AlertDialog::builder()
                .heading(gettext("Files Are Still Uploading"))
                .body(message)
                .build();
            dialog.add_response("wait", &gettext("_Wait"));
            dialog.add_response("now", &gettext("_Quit Now"));
            dialog.set_response_appearance("now", adw::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("wait"));
            let response = dialog.choose_future(Some(&ui.window)).await;
            if response != "now" {
                return false;
            }
            let _ = ui.backend.call(Command::DaemonStop { now: true }).await;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Request;

    fn parse(args: &[&str]) -> Request {
        Request::parse(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn requests() {
        assert_eq!(parse(&[]), Request::Window);
        assert_eq!(parse(&["--background"]), Request::Background);
        assert_eq!(
            parse(&["--action", "preferences"]),
            Request::Action("preferences".into(), None)
        );
        assert_eq!(
            parse(&["--action", "reconnect", "--target", "abc"]),
            Request::Action("reconnect".into(), Some("abc".into()))
        );
        // An action without its name is a plain launch.
        assert_eq!(parse(&["--action"]), Request::Window);
    }
}
