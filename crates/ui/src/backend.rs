//! The link between the GTK interface and `nuaged`.
//!
//! GTK runs on the GLib main loop; the IPC client needs tokio. A thread
//! runs a small tokio runtime: the interface sends it commands and awaits
//! the replies (`async-channel` works on both sides), and it forwards the
//! daemon's events, reconnecting when the daemon restarts.

use std::path::PathBuf;

use nuage_core::{Command, Error, Reply};
use nuage_ipc::Client;

/// What the interface hears from the daemon. On `Connected` it reloads its
/// state.
pub use nuage_tray::daemon::Link as BackendEvent;

/// An answer, or why there is none.
pub type Answer = Result<Reply, String>;

/// Work run on the backend's tokio runtime.
type Job =
    Box<dyn FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> + Send>;

/// Handle to the backend thread. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Backend {
    jobs: async_channel::Sender<Job>,
    sockets: Vec<PathBuf>,
}

fn client_name() -> String {
    format!("nuage-gtk {}", nuage_core::VERSION)
}

/// Turns any failure into a message for the user.
fn message(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Backend {
    /// Starts the backend thread. Events arrive on the returned receiver.
    pub fn start() -> std::io::Result<(Self, async_channel::Receiver<BackendEvent>)> {
        let sockets = nuage_ipc::candidate_socket_paths().map_err(std::io::Error::other)?;
        let (jobs_tx, jobs_rx) = async_channel::unbounded::<Job>();
        let (events_tx, events_rx) = async_channel::unbounded();
        let event_sockets = sockets.clone();
        std::thread::Builder::new()
            .name("nuage-backend".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        tracing::error!(error = %e, "cannot start the backend runtime");
                        return;
                    }
                };
                runtime.block_on(async move {
                    tokio::spawn(nuage_tray::daemon::follow(
                        event_sockets,
                        client_name(),
                        events_tx,
                    ));
                    while let Ok(job) = jobs_rx.recv().await {
                        tokio::spawn(job());
                    }
                });
            })?;
        Ok((
            Self {
                jobs: jobs_tx,
                sockets,
            },
            events_rx,
        ))
    }

    /// Runs `future` on the backend's tokio runtime and returns its output.
    pub async fn run<F, T>(&self, future: F) -> T
    where
        F: std::future::Future<Output = T> + Send + 'static,
        T: Send + 'static,
    {
        let (tx, rx) = async_channel::bounded(1);
        let job: Job = Box::new(move || {
            Box::pin(async move {
                let _ = tx.send(future.await).await;
            })
        });
        if self.jobs.send(job).await.is_err() {
            panic!("the backend thread stopped");
        }
        rx.recv().await.expect("the backend thread stopped")
    }

    /// Sends a command to the daemon and waits for its reply.
    pub async fn call(&self, command: Command) -> Answer {
        match self.request(command).await {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(e)) => Err(e.to_string()),
            Err(e) => Err(e),
        }
    }

    /// [`call`](Self::call), keeping the daemon's error (its code tells,
    /// e.g., that a confirmation is needed). The outer error means that
    /// the daemon could not be reached.
    pub async fn request(&self, command: Command) -> Result<Result<Reply, Error>, String> {
        let sockets = self.sockets.clone();
        self.run(async move {
            let mut client = Client::connect_any(&sockets, &client_name())
                .await
                .map_err(message)?;
            client.request(command).await.map_err(message)
        })
        .await
    }

    /// The version of the running daemon, if it answers.
    pub async fn daemon_version(&self) -> Option<String> {
        let sockets = self.sockets.clone();
        self.run(async move {
            let client = Client::connect_any(&sockets, &client_name()).await.ok()?;
            Some(client.daemon_version().to_owned())
        })
        .await
    }

    /// Whether the daemon answers.
    pub async fn daemon_running(&self) -> bool {
        let sockets = self.sockets.clone();
        self.run(async move { Client::connect_any(&sockets, &client_name()).await.is_ok() })
            .await
    }

    /// Starts `nuaged` if it does not run, and waits until it answers.
    ///
    /// The daemon is started detached: closing the application does not
    /// stop the synchronisation. It is looked for next to this program,
    /// then in `PATH`.
    pub async fn ensure_daemon(&self) -> Result<(), String> {
        let sockets = self.sockets.clone();
        self.run(async move { nuage_tray::daemon::ensure(&sockets, &client_name()).await })
            .await
    }

    /// Starts the application at login, or stops doing so. Returns whether
    /// it now starts at login.
    ///
    /// In the Flatpak, asks the Background portal. Outside of it, the portal
    /// refuses (it only knows sandboxed applications): an XDG autostart
    /// entry is written instead.
    pub async fn request_autostart(&self, enable: bool) -> Result<bool, String> {
        if !nuage_tray::updates::in_flatpak() {
            return set_autostart_entry(enable).map(|()| enable);
        }
        self.run(async move {
            let request = ashpd::desktop::background::Background::request()
                .reason("Keep your cloud folders mounted and synchronised.")
                .auto_start(enable)
                // The tray agent only: no window, and no GTK, at login.
                .command([nuage_tray::agent::PROGRAM])
                .dbus_activatable(false)
                .send()
                .await
                .map_err(message)?;
            let response = request.response().map_err(message)?;
            Ok(response.auto_start())
        })
        .await
    }

    /// Starts the newest installed version of Nuage once this instance has
    /// quit (its D-Bus name is free; GApplication would hand the launch back
    /// to it otherwise).
    pub async fn start_latest_version(&self) -> Result<(), String> {
        self.run(async move {
            use ashpd::flatpak::{Flatpak, SpawnFlags, SpawnOptions};
            let flatpak = Flatpak::new().await.map_err(message)?;
            flatpak
                .spawn(
                    "/",
                    &["sh", "-c", RELAUNCH],
                    std::collections::HashMap::<u32, std::os::fd::OwnedFd>::new(),
                    std::collections::HashMap::new(),
                    SpawnFlags::LatestVersion.into(),
                    SpawnOptions::default(),
                )
                .await
                .map(|_| ())
                .map_err(message)
        })
        .await
    }
}

/// Waits (up to 30 s) for the running instance to release the
/// application's name, then starts the application.
const RELAUNCH: &str = concat!(
    "for i in $(seq 60); do ",
    "gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus ",
    "--method org.freedesktop.DBus.NameHasOwner io.github.felixilgatto.Nuage ",
    "| grep -q true || break; sleep 0.5; done; exec nuage-gtk"
);

/// `~/.config/autostart/<app id>.desktop`, running this executable with
/// `--background`.
fn set_autostart_entry(enable: bool) -> Result<(), String> {
    let path = gtk::glib::user_config_dir()
        .join("autostart")
        .join(format!("{}.desktop", nuage_core::APP_ID));
    if !enable {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("Cannot remove {}: {e}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().map_err(message)?;
    // Desktop entry quoting: spaces and specials need double quotes.
    let exec = format!(
        "\"{}\" --background",
        exe.display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`")
    );
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Nuage\nComment=Keep your cloud folders mounted and synced\nExec={exec}\nIcon={}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
        nuage_core::APP_ID
    );
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(&path, entry).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

pub use nuage_tray::daemon::log_file;
