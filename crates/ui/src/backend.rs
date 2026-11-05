//! The link between the GTK interface and `nuaged`.
//!
//! GTK runs on the GLib main loop; the IPC client needs tokio. A thread
//! runs a small tokio runtime: the interface sends it commands and awaits
//! the replies (`async-channel` works on both sides), and it forwards the
//! daemon's events, reconnecting when the daemon restarts.

use std::path::PathBuf;
use std::time::Duration;

use nuage_core::{Command, Error, Event, Reply};
use nuage_ipc::{Client, IpcError};

/// What the interface hears from the daemon.
#[derive(Debug, Clone)]
pub enum BackendEvent {
    /// Connected (again): the interface should reload its state.
    Connected,
    /// The daemon cannot be reached.
    Disconnected(String),
    Event(Event),
}

/// What the Flatpak update monitor reports about Nuage itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    /// A newer version is available in the repository, not installed yet.
    Available,
    /// A newer version is installed: this instance and its daemon keep
    /// running the old one until restarted.
    Installed,
    /// Installing the update failed, with the portal's message.
    Failed(String),
}

/// What a commit triple of `UpdateAvailable` means.
fn classify_update(running: &str, local: &str, remote: &str) -> Option<UpdateEvent> {
    if !local.is_empty() && local != running {
        Some(UpdateEvent::Installed)
    } else if !remote.is_empty() && remote != running {
        Some(UpdateEvent::Available)
    } else {
        None
    }
}

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
    /// Requests to install the available update, read by [`Self::watch_updates`].
    install: (async_channel::Sender<()>, async_channel::Receiver<()>),
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
                    tokio::spawn(forward_events(event_sockets, events_tx));
                    while let Ok(job) = jobs_rx.recv().await {
                        tokio::spawn(job());
                    }
                });
            })?;
        Ok((
            Self {
                jobs: jobs_tx,
                sockets,
                install: async_channel::bounded(1),
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
        if self.daemon_running().await {
            return Ok(());
        }
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.join("nuaged")))
            .filter(|p| p.is_file())
            .unwrap_or_else(|| PathBuf::from("nuaged"));
        let log = log_file();
        let started = self
            .run(async move {
                // The log of the previous run is kept: it says why it stopped.
                if let Some(log) = &log {
                    let _ = std::fs::rename(log, log.with_extension("log.old"));
                }
                let stderr = match log.as_ref().map(std::fs::File::create) {
                    Some(Ok(file)) => std::process::Stdio::from(file),
                    _ => std::process::Stdio::null(),
                };
                tokio::process::Command::new(&program)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(stderr)
                    // The daemon outlives the application.
                    .kill_on_drop(false)
                    .spawn()
                    .map(drop)
                    .map_err(|e| format!("Cannot start {}: {e}", program.display()))
            })
            .await;
        started?;
        for _ in 0..100 {
            if self.daemon_running().await {
                return Ok(());
            }
            glib_sleep(Duration::from_millis(100)).await;
        }
        Err(match log_file() {
            Some(log) => format!(
                "The Nuage service did not start. Its log is in {}.",
                log.display()
            ),
            None => "The Nuage service did not start.".to_owned(),
        })
    }

    /// Starts the application at login, or stops doing so. Returns whether
    /// it now starts at login.
    ///
    /// In the Flatpak, asks the Background portal. Outside of it, the portal
    /// refuses (it only knows sandboxed applications): an XDG autostart
    /// entry is written instead.
    pub async fn request_autostart(&self, enable: bool) -> Result<bool, String> {
        if !in_flatpak() {
            return set_autostart_entry(enable).map(|()| enable);
        }
        self.run(async move {
            let request = ashpd::desktop::background::Background::request()
                .reason("Keep your cloud folders mounted and synchronised.")
                .auto_start(enable)
                .command(["nuage-gtk", "--background"])
                .dbus_activatable(false)
                .send()
                .await
                .map_err(message)?;
            let response = request.response().map_err(message)?;
            Ok(response.auto_start())
        })
        .await
    }

    /// In the Flatpak, reports updates of Nuage itself on the returned
    /// channel, and installs the available one on [`Self::install_update`].
    /// Outside the Flatpak or without the portal, nothing is ever reported.
    pub fn watch_updates(&self) -> async_channel::Receiver<UpdateEvent> {
        let (tx, rx) = async_channel::unbounded();
        if !in_flatpak() {
            return rx;
        }
        let install = self.install.1.clone();
        let backend = self.clone();
        gtk::glib::spawn_future_local(async move {
            let result = backend
                .run(async move { monitor_updates(tx, install).await })
                .await;
            if let Err(e) = result {
                tracing::info!(error = %e, "no update monitor");
            }
        });
        rx
    }

    /// Asks the update monitor to install the available update. The outcome
    /// arrives on [`Self::watch_updates`]'s channel.
    pub fn install_update(&self) {
        let _ = self.install.0.try_send(());
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

/// Runs the Flatpak update monitor until it closes.
async fn monitor_updates(
    tx: async_channel::Sender<UpdateEvent>,
    install: async_channel::Receiver<()>,
) -> Result<(), String> {
    use ashpd::flatpak::{Flatpak, UpdateStatus};
    use futures_util::StreamExt;
    let flatpak = Flatpak::new().await.map_err(message)?;
    let monitor = flatpak.create_update_monitor().await.map_err(message)?;
    let mut available = monitor.receive_update_available().await.map_err(message)?;
    let mut progress = monitor.receive_progress().await.map_err(message)?;
    loop {
        tokio::select! {
            info = available.next() => {
                let Some(info) = info else { break };
                tracing::info!(
                    running = info.running_commit(),
                    local = info.local_commit(),
                    remote = info.remote_commit(),
                    "update monitor"
                );
                let event = classify_update(
                    info.running_commit(),
                    info.local_commit(),
                    info.remote_commit(),
                );
                if let Some(event) = event {
                    let _ = tx.send(event).await;
                }
            }
            p = progress.next() => {
                let Some(p) = p else { break };
                let event = match p.status {
                    Some(UpdateStatus::Done) => UpdateEvent::Installed,
                    Some(UpdateStatus::Failed) => UpdateEvent::Failed(
                        p.error_message.or(p.error).unwrap_or_default(),
                    ),
                    _ => continue,
                };
                let _ = tx.send(event).await;
            }
            request = install.recv() => {
                if request.is_err() {
                    break;
                }
                if let Err(e) = monitor.update(None).await {
                    let _ = tx.send(UpdateEvent::Failed(e.to_string())).await;
                }
            }
        }
    }
    Err("the update monitor closed".to_owned())
}

fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

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

/// Where the daemon started by the application writes its log.
pub fn log_file() -> Option<PathBuf> {
    let dir = gtk::glib::user_data_dir().join("nuage");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("nuaged.log"))
}

/// Sleeps on the GLib main loop.
pub async fn glib_sleep(duration: Duration) {
    gtk::glib::timeout_future(duration).await;
}

/// Subscribes to the daemon's events, forever, reconnecting as needed.
async fn forward_events(sockets: Vec<PathBuf>, tx: async_channel::Sender<BackendEvent>) {
    let mut connected = None;
    loop {
        match Client::connect_any(&sockets, &client_name()).await {
            Ok(client) => match client.subscribe().await {
                Ok(mut events) => {
                    if tx.send(BackendEvent::Connected).await.is_err() {
                        return;
                    }
                    connected = Some(true);
                    loop {
                        match events.next_event().await {
                            Ok(Some(event)) => {
                                if tx.send(BackendEvent::Event(event)).await.is_err() {
                                    return;
                                }
                            }
                            Ok(None) | Err(IpcError::ConnectionClosed) => break,
                            Err(e) => {
                                tracing::debug!(error = %e, "event stream failed");
                                break;
                            }
                        }
                    }
                }
                Err(e) => tracing::debug!(error = %e, "cannot subscribe"),
            },
            Err(e) => {
                if connected != Some(false) {
                    connected = Some(false);
                    if tx
                        .send(BackendEvent::Disconnected(e.to_string()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }
        if connected == Some(true) {
            connected = None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::{UpdateEvent, classify_update};

    #[test]
    fn update_states() {
        // Nothing new.
        assert_eq!(classify_update("a", "a", "a"), None);
        assert_eq!(classify_update("a", "a", ""), None);
        // In the repository only.
        assert_eq!(classify_update("a", "a", "b"), Some(UpdateEvent::Available));
        // Installed while running, whatever the repository has.
        assert_eq!(classify_update("a", "b", "b"), Some(UpdateEvent::Installed));
        assert_eq!(classify_update("a", "b", "c"), Some(UpdateEvent::Installed));
    }
}
