//! `nuage-gtk`: the Nuage desktop application (window + tray icon).
//!
//! A client of `nuaged`, which it starts if needed. Launched by the user,
//! it opens the setup (no account yet) or the main window; closing the
//! window leaves it in the system tray. `--background` (used to start at
//! login) opens nothing when the tray is available.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use nuage_core::{Command, Event, StatusReport};
use nuage_tray::{NuageTray, TrayAction};
use nuage_ui::{Backend, BackendEvent, Ui};

/// Where translations are installed (overridable for development).
const DEFAULT_LOCALE_DIR: &str = "/app/share/locale";

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
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

    let mut args: Vec<String> = std::env::args().collect();
    let background = args.iter().any(|a| a == "--background");
    args.retain(|a| a != "--background");

    let app = adw::Application::builder()
        .application_id(nuage_core::APP_ID)
        .build();

    // The first activation is decided by `startup`; later ones (the user
    // launches the application again) show the window.
    let first = Rc::new(Cell::new(true));
    app.connect_activate(move |_| {
        if first.replace(false) {
            return;
        }
        if let Some(ui) = current_ui() {
            ui.present();
        }
    });
    app.connect_startup(move |app| startup(app, background));
    app.run_with_args(&args)
}

fn startup(app: &adw::Application, background: bool) {
    let (backend, events) = match Backend::start() {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "cannot start");
            return;
        }
    };
    let ui = Ui::new(app, backend);
    UI.with(|u| *u.borrow_mut() = Some(Rc::clone(&ui)));
    add_actions(app, &ui);

    // Keep running with the window closed while the tray icon shows.
    let hold = Rc::new(RefCell::new(Some(app.hold())));
    {
        let (app, ui2, hold) = (app.clone(), Rc::clone(&ui), Rc::clone(&hold));
        ui.window.connect_close_request(move |_| {
            if !ui2.tray_available() {
                hold.borrow_mut().take();
                app.quit();
            }
            glib::Propagation::Proceed
        });
    }

    let (tray_actions_tx, tray_actions) = async_channel::unbounded::<TrayAction>();
    let (tray_status, tray_status_rx) = async_channel::unbounded::<Option<StatusReport>>();
    let (tray_ok_tx, tray_ok) = async_channel::bounded::<bool>(1);
    start_tray(tray_actions_tx, tray_status_rx, tray_ok_tx);

    // Startup: daemon, tray, then window or not.
    {
        let ui = Rc::clone(&ui);
        glib::spawn_future_local(async move {
            if let Err(e) = ui.backend.ensure_daemon().await {
                ui.show_disconnected(&e);
                ui.present();
                return;
            }
            let tray = glib::future_with_timeout(Duration::from_secs(3), tray_ok.recv())
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or(false);
            ui.set_tray_available(tray);
            ui.reload().await;
            if background && tray {
                return;
            }
            if ui.accounts().is_empty() {
                nuage_ui::onboarding::start(&ui);
            } else {
                ui.present();
            }
        });
    }

    // Events from the daemon.
    {
        let (ui, tray_status) = (Rc::clone(&ui), tray_status.clone());
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                match event {
                    BackendEvent::Connected => {
                        ui.reload().await;
                        let _ = tray_status.send(ui.status()).await;
                    }
                    BackendEvent::Disconnected(message) => {
                        ui.show_disconnected(&message);
                        let _ = tray_status.send(None).await;
                    }
                    BackendEvent::Event(Event::StatusChanged { status }) => {
                        let _ = tray_status.send(Some(status.clone())).await;
                        ui.show_status(status);
                    }
                    BackendEvent::Event(Event::DaemonStopping) => {
                        let _ = tray_status.send(None).await;
                    }
                    BackendEvent::Event(_) => {}
                }
            }
        });
    }

    // Tray menu.
    {
        let app = app.clone();
        glib::spawn_future_local(async move {
            let _hold = hold;
            while let Ok(action) = tray_actions.recv().await {
                match action {
                    TrayAction::OpenWindow => {
                        if let Some(ui) = current_ui() {
                            ui.present();
                        }
                    }
                    TrayAction::OpenFolder(path) => open_folder(&path),
                    TrayAction::Refresh => app.activate_action("refresh", None),
                    TrayAction::PauseAll => app.activate_action("pause-all", None),
                    TrayAction::ResumeAll => app.activate_action("resume-all", None),
                    TrayAction::AddAccount => app.activate_action("add-account", None),
                    TrayAction::Quit => app.activate_action("quit-all", None),
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
    // Notification button: open a folder.
    let open = gio::SimpleAction::new("open-folder", Some(glib::VariantTy::STRING));
    open.connect_activate(|_, param| {
        if let Some(path) = param.and_then(|p| p.get::<String>()) {
            open_folder(&PathBuf::from(path));
        }
    });
    app.add_action(&open);
}

/// "Quit": stops the application and the daemon, following the "quit with
/// pending uploads" setting.
async fn quit(app: &adw::Application, ui: &Rc<Ui>) {
    match ui.backend.call(Command::DaemonStop { now: false }).await {
        Ok(_) => app.quit(),
        // Setting "ask": uploads are pending.
        Err(message) => {
            if !ui.backend.daemon_running().await {
                app.quit();
                return;
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
            if response == "now" {
                let _ = ui.backend.call(Command::DaemonStop { now: true }).await;
                app.quit();
            }
        }
    }
}

/// Runs the tray icon on its own thread with a small tokio runtime. Says on
/// `available` whether a system tray shows it.
fn start_tray(
    actions: async_channel::Sender<TrayAction>,
    status: async_channel::Receiver<Option<StatusReport>>,
    available: async_channel::Sender<bool>,
) {
    let spawned = std::thread::Builder::new()
        .name("tray".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot start the tray runtime");
                    let _ = available.send_blocking(false);
                    return;
                }
            };
            runtime.block_on(async move {
                let tray = NuageTray::new(move |action| {
                    let _ = actions.send_blocking(action);
                });
                match tray.spawn().await {
                    Ok(handle) => {
                        let _ = available.send(true).await;
                        while let Ok(report) = status.recv().await {
                            handle.update(|t| t.set_status(report)).await;
                        }
                        std::future::pending::<()>().await
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "no system tray available");
                        let _ = available.send(false).await;
                    }
                }
            });
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "cannot start the tray thread");
    }
}
