//! `nuage-gtk`: the Nuage desktop application (window + tray icon).
//!
//! A client of `nuaged`. **Skeleton**: opens an empty window and shows the
//! tray icon when a StatusNotifierItem host is available.

use adw::prelude::*;
use gtk::glib;

use nuage_tray::{NuageTray, TrayAction};

/// Where translations are installed (overridable for development).
const DEFAULT_LOCALE_DIR: &str = "/app/share/locale";

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
        .build();

    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            nuage_ui::build_main_window(app).present();
        }
    });

    app.connect_startup(|app| {
        // Keep running in the background while the tray icon is shown.
        let hold = app.hold();
        let (tx, rx) = async_channel::unbounded::<TrayAction>();
        start_tray(tx);

        let app = app.clone();
        glib::spawn_future_local(async move {
            let _hold = hold;
            while let Ok(action) = rx.recv().await {
                match action {
                    TrayAction::OpenWindow => app.activate(),
                    TrayAction::Quit => app.quit(),
                }
            }
        });
    });

    app.run()
}

/// Runs the tray icon on its own thread with a small tokio runtime.
fn start_tray(tx: async_channel::Sender<TrayAction>) {
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
                    return;
                }
            };
            runtime.block_on(async move {
                let tray = NuageTray::new(move |action| {
                    let _ = tx.send_blocking(action);
                });
                match tray.spawn().await {
                    // Keep the handle alive for as long as the application runs.
                    Ok(_handle) => std::future::pending::<()>().await,
                    Err(e) => tracing::warn!(error = %e, "no system tray available"),
                }
            });
        });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "cannot start the tray thread");
    }
}
