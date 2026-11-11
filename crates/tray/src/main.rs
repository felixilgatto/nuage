//! `nuage-tray`: the part of the Nuage desktop application that stays
//! running when its window is closed. No GTK, so it stays small.
//!
//! It keeps the daemon running (starts it, and again if it crashes), shows
//! the tray icon, sends the desktop notifications and offers the updates
//! of Nuage itself. What needs a window (preferences, signing in again,
//! adding an account, a question before quitting) opens `nuage-gtk`.
//!
//! One agent per session (a lock next to the daemon's socket). It ends
//! when the daemon stops (the user quit Nuage) or when the window's "Quit"
//! or "Restart" stops it.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ashpd::desktop::notification::{Button, Notification, NotificationProxy, Priority};
use futures_util::{Stream, StreamExt};
use gettextrs::gettext;
use nuage_core::{AccountId, Command, Event, PinState, Reply, StatusReport};
use nuage_ipc::{Client, IpcError};
use nuage_tray::updates::{self, UpdateEvent};
use nuage_tray::{NuageTray, TrayAction, agent, daemon};

/// Minimum delay between two restarts of a crashed daemon.
const RESTART_INTERVAL: Duration = Duration::from_secs(60);
/// How long to wait for a previous agent to end (a restart after an update).
const CLAIM_WAIT: Duration = Duration::from_secs(10);
/// Where translations are installed (overridable for development).
const DEFAULT_LOCALE_DIR: &str = "/app/share/locale";
/// Notification of the update of Nuage, one at a time.
const UPDATE_NOTIFICATION: &str = "update";

fn client_name() -> String {
    format!("nuage-tray {}", nuage_core::VERSION)
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NUAGE_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let locale_dir =
        std::env::var("NUAGE_LOCALE_DIR").unwrap_or_else(|_| DEFAULT_LOCALE_DIR.to_owned());
    // `setlocale` (done by GTK in the window) is `unsafe`, which the
    // workspace forbids: strings stay in English here until translations
    // exist and a safe way to set the locale is found.
    let _ = gettextrs::bindtextdomain("nuage", locale_dir);
    let _ = gettextrs::bind_textdomain_codeset("nuage", "UTF-8");
    let _ = gettextrs::textdomain("nuage");

    let _instance = match agent::claim(CLAIM_WAIT) {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            tracing::info!("the Nuage tray agent already runs");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            tracing::error!(error = %e, "cannot claim the tray agent's lock");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "cannot start the runtime");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = %e, "the tray agent stopped");
            ExitCode::FAILURE
        }
    }
}

/// What the agent hears from the daemon.
#[derive(Debug)]
enum Link {
    Connected,
    Disconnected,
    Event(Event),
}

/// Where the update of Nuage stands, so each step is offered once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateNotice {
    None,
    Available,
    Installing,
    Installed,
}

/// The icons of a source build (`data/icons`); installed icons are found
/// in the system icon theme.
fn source_icon_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons"));
    dir.is_dir().then_some(dir)
}

type Actions = Pin<Box<dyn Stream<Item = ashpd::desktop::notification::Action> + Send>>;

struct Agent {
    sockets: Vec<PathBuf>,
    tray: Option<ksni::Handle<NuageTray>>,
    notifications: Option<NotificationProxy<'static>>,
    status: Option<StatusReport>,
    update: UpdateNotice,
    install: async_channel::Sender<()>,
}

async fn run() -> Result<(), String> {
    let sockets = nuage_ipc::candidate_socket_paths().map_err(|e| e.to_string())?;
    if let Err(e) = daemon::ensure(&sockets, &client_name()).await {
        tracing::warn!(error = %e, "cannot start the Nuage service");
    }

    let (actions_tx, actions) = async_channel::unbounded::<TrayAction>();
    let mut icon = NuageTray::new(move |action| {
        let _ = actions_tx.try_send(action);
    });
    if let Some(dir) = source_icon_dir() {
        icon = icon.with_icon_theme_path(dir.to_string_lossy());
    }
    let tray = match icon.spawn().await {
        Ok(handle) => Some(handle),
        Err(e) => {
            // Without an icon nothing would show that Nuage runs (GNOME
            // without the AppIndicator extension): open the window.
            tracing::warn!(error = %e, "no system tray available; running without an icon");
            open_window(&[]);
            None
        }
    };

    let notifications = match NotificationProxy::new().await {
        Ok(proxy) => Some(proxy),
        Err(e) => {
            tracing::warn!(error = %e, "no notification portal");
            None
        }
    };
    let mut invoked: Actions = match &notifications {
        Some(proxy) => match proxy.receive_action_invoked().await {
            Ok(stream) => Box::pin(stream),
            Err(e) => {
                tracing::warn!(error = %e, "cannot follow notification buttons");
                Box::pin(futures_util::stream::pending())
            }
        },
        None => Box::pin(futures_util::stream::pending()),
    };

    let (update_tx, update_events) = async_channel::unbounded();
    let (install, install_rx) = async_channel::bounded(1);
    if updates::in_flatpak() {
        tokio::spawn(async move {
            if let Err(e) = updates::monitor(update_tx, install_rx).await {
                tracing::info!(error = %e, "no update monitor");
            }
        });
    }

    let (link_tx, link) = async_channel::unbounded();
    tokio::spawn(follow_daemon(sockets.clone(), link_tx));

    let mut agent = Agent {
        sockets,
        tray,
        notifications,
        status: None,
        update: UpdateNotice::None,
        install,
    };
    // A daemon that was connected and goes away without saying it stops
    // has crashed: it is started again, at most once a minute.
    let mut connected = false;
    let mut stopping = false;
    let mut last_restart: Option<Instant> = None;
    loop {
        tokio::select! {
            l = link.recv() => match l {
                Ok(Link::Connected) => {
                    (connected, stopping) = (true, false);
                    if let Ok(Ok(Reply::Status(status))) = agent.request(Command::Status).await {
                        agent.set_status(Some(status)).await;
                    }
                }
                Ok(Link::Disconnected) => {
                    agent.set_status(None).await;
                    if stopping {
                        tracing::info!("the Nuage service stopped; the tray agent ends");
                        return Ok(());
                    }
                    let crashed = std::mem::take(&mut connected);
                    if crashed && last_restart.is_none_or(|t| t.elapsed() > RESTART_INTERVAL) {
                        tracing::warn!("the Nuage service stopped; starting it again");
                        last_restart = Some(Instant::now());
                        if let Err(e) = daemon::ensure(&agent.sockets, &client_name()).await {
                            tracing::warn!(error = %e, "cannot start the Nuage service again");
                        }
                    }
                }
                Ok(Link::Event(Event::StatusChanged { status })) => {
                    agent.set_status(Some(status)).await;
                }
                Ok(Link::Event(Event::DaemonStopping)) => {
                    stopping = true;
                    agent.set_status(None).await;
                }
                Ok(Link::Event(event)) => agent.notify_event(&event).await,
                Err(_) => return Err("the daemon link ended".to_owned()),
            },
            Ok(action) = actions.recv() => {
                if agent.tray_action(action).await {
                    return Ok(());
                }
            }
            Some(action) = invoked.next() => {
                let target = action
                    .parameter()
                    .first()
                    .and_then(|v| String::try_from(v.clone()).ok());
                agent.notification_action(action.name(), target).await;
            }
            Ok(event) = update_events.recv() => agent.update_event(event).await,
        }
    }
}

impl Agent {
    async fn request(&self, command: Command) -> Result<Result<Reply, nuage_core::Error>, String> {
        let mut client = Client::connect_any(&self.sockets, &client_name())
            .await
            .map_err(|e| e.to_string())?;
        client.request(command).await.map_err(|e| e.to_string())
    }

    async fn set_status(&mut self, status: Option<StatusReport>) {
        self.status.clone_from(&status);
        if let Some(tray) = &self.tray {
            tray.update(|t| t.set_status(status)).await;
        }
    }

    /// What an account is called, from the last status.
    fn name(&self, id: &AccountId) -> String {
        self.status
            .iter()
            .flat_map(|s| &s.accounts)
            .find(|a| &a.account.id == id)
            .map_or_else(
                || id.as_str().to_owned(),
                |a| a.account.display_name.clone(),
            )
    }

    /// A menu item. Returns whether the agent should end.
    async fn tray_action(&mut self, action: TrayAction) -> bool {
        match action {
            TrayAction::OpenWindow => open_window(&[]),
            TrayAction::OpenFolder(path) => open_folder(&path).await,
            TrayAction::AddAccount => open_window(&["--action", "add-account"]),
            TrayAction::Preferences => open_window(&["--action", "preferences"]),
            TrayAction::Refresh => self.command(Command::Refresh { account: None }).await,
            TrayAction::PauseAll => self.command(Command::Pause { account: None }).await,
            TrayAction::ResumeAll => self.command(Command::Resume { account: None }).await,
            TrayAction::Quit => return self.quit().await,
        }
        false
    }

    /// Runs a command; says what went wrong, if anything.
    async fn command(&self, command: Command) {
        let error = match self.request(command).await {
            Ok(Ok(_)) => return,
            Ok(Err(e)) => e.to_string(),
            Err(e) => e,
        };
        self.send("command", Notification::new("Nuage").body(error.as_str()))
            .await;
    }

    /// "Quit": stops the daemon, following the "quit with pending uploads"
    /// setting. If it asks first, the window asks the user. Whether the
    /// agent should end.
    async fn quit(&self) -> bool {
        match self.request(Command::DaemonStop { now: false }).await {
            Ok(Err(_)) => {
                open_window(&["--action", "quit-all"]);
                false
            }
            // Stopped, stopping once the uploads are done, or not running.
            Ok(Ok(_)) | Err(_) => true,
        }
    }

    /// A notification button (or the notification itself) was clicked.
    async fn notification_action(&mut self, name: &str, target: Option<String>) {
        let target = target.unwrap_or_default();
        match name {
            "reconnect" | "account-preferences" => {
                open_window(&["--action", name, "--target", &target]);
            }
            "open-folder" => open_folder(Path::new(&target)).await,
            "install-update" => {
                if self.update != UpdateNotice::Installing {
                    self.update = UpdateNotice::Installing;
                    self.withdraw(UPDATE_NOTIFICATION).await;
                    let _ = self.install.try_send(());
                }
            }
            "restart" => open_window(&["--action", "restart"]),
            _ => open_window(&[]),
        }
    }

    async fn send(&self, id: &str, notification: Notification) {
        let Some(proxy) = &self.notifications else {
            return;
        };
        if let Err(e) = proxy.add_notification(id, notification).await {
            tracing::warn!(error = %e, "cannot send a notification");
        }
    }

    async fn withdraw(&self, id: &str) {
        if let Some(proxy) = &self.notifications {
            let _ = proxy.remove_notification(id).await;
        }
    }

    /// Desktop notifications for what needs the user.
    async fn notify_event(&self, event: &Event) {
        let (key, notification) = match event {
            Event::AuthRequired { account } => {
                let title = gettext("Sign in to {} again").replace("{}", &self.name(account));
                let body = gettext(
                    "Its access expired or was revoked: files are not synced until you sign in.",
                );
                let n = Notification::new(&title)
                    .body(body.as_str())
                    .priority(Priority::High)
                    .default_action("reconnect")
                    .default_action_target(account.as_str())
                    .button(Button::new(&gettext("Sign In"), "reconnect").target(account.as_str()));
                (format!("auth-{account}"), n)
            }
            Event::UploadFailed {
                account, message, ..
            } => {
                let n = Notification::new(&gettext("Some files could not be uploaded"))
                    .body(message.as_str())
                    .default_action("open-window");
                (format!("upload-{account}"), n)
            }
            Event::CacheFull { account } => {
                let body = gettext(
                    "Files of {} cannot be cached or uploaded. Free some space, or lower its cache limit.",
                )
                .replace("{}", &self.name(account));
                let n = Notification::new(&gettext("The disk is full"))
                    .body(body.as_str())
                    .default_action("open-window")
                    .button(
                        Button::new(&gettext("Settings"), "account-preferences")
                            .target(account.as_str()),
                    );
                (format!("cache-{account}"), n)
            }
            Event::OfflineSpaceWarning {
                account,
                used,
                limit,
            } => {
                let body = gettext(
                    "{account} keeps {used} on this computer (warning above {limit}). Remove offline copies you no longer need.",
                )
                .replace("{account}", &self.name(account))
                .replace("{used}", &nuage_tray::human_size(*used))
                .replace("{limit}", &nuage_tray::human_size(*limit));
                let n = Notification::new(&gettext("Offline items take a lot of space"))
                    .body(body.as_str())
                    .default_action("open-window")
                    .button(
                        Button::new(&gettext("Settings"), "account-preferences")
                            .target(account.as_str()),
                    );
                (format!("offline-space-{account}"), n)
            }
            Event::PinStateChanged {
                account,
                path,
                state: PinState::Conflict,
            } => {
                let body = gettext(
                    "{file} was changed on this computer and in {account}. Both versions are kept: yours is now “{copy}”.",
                )
                .replace("{file}", path.split(" (conflict ").next().unwrap_or(path))
                .replace("{account}", &self.name(account))
                .replace("{copy}", path.rsplit('/').next().unwrap_or(path));
                let n = Notification::new(&gettext("A file changed in two places"))
                    .body(body.as_str())
                    .default_action("open-window");
                (format!("conflict-{account}-{path}"), n)
            }
            _ => return,
        };
        self.send(&key, notification).await;
    }

    /// Tells the user about an update of Nuage itself, once per step.
    async fn update_event(&mut self, event: UpdateEvent) {
        match event {
            UpdateEvent::Available if self.update == UpdateNotice::None => {
                let body = gettext(
                    "Update now, or later from Software. Your folders stay mounted while it downloads.",
                );
                let n = Notification::new(&gettext("A new version of Nuage is available"))
                    .body(body.as_str())
                    .default_action("open-window")
                    .button(Button::new(&gettext("Update"), "install-update"));
                self.send(UPDATE_NOTIFICATION, n).await;
                self.update = UpdateNotice::Available;
            }
            UpdateEvent::Installed if self.update != UpdateNotice::Installed => {
                let body =
                    gettext("Restart Nuage to use it. Your folders are unmounted for a moment.");
                let n = Notification::new(&gettext("A new version of Nuage is installed"))
                    .body(body.as_str())
                    .default_action("open-window")
                    .button(Button::new(&gettext("Restart"), "restart"));
                self.send(UPDATE_NOTIFICATION, n).await;
                self.update = UpdateNotice::Installed;
            }
            // The portal's message is technical: it goes to the log only.
            UpdateEvent::Failed(error) => {
                tracing::warn!(error, "cannot install the update");
                let body = gettext("Update it from Software instead.");
                let n = Notification::new(&gettext("Nuage could not update itself"))
                    .body(body.as_str())
                    .default_action("open-window");
                self.send(UPDATE_NOTIFICATION, n).await;
                // Offered again if the monitor reports the update again.
                self.update = UpdateNotice::None;
            }
            _ => {}
        }
    }
}

/// Opens the window (`nuage-gtk`), with an action to run in it. A window
/// already open gets the action. Started through tokio, which reaps it
/// when it ends.
fn open_window(args: &[&str]) {
    let program = daemon::sibling("nuage-gtk");
    if let Err(e) = tokio::process::Command::new(&program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .spawn()
    {
        tracing::warn!(error = %e, program = %program.display(), "cannot open the window");
    }
}

/// Opens a folder in the file manager (OpenURI portal).
async fn open_folder(path: &Path) {
    let result = match std::fs::File::open(path) {
        Ok(dir) => ashpd::desktop::open_uri::OpenFileRequest::default()
            .send_file(&dir)
            .await
            .map(drop)
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = result {
        tracing::warn!(error = %e, path = %path.display(), "cannot open the folder");
    }
}

/// Follows the daemon's events, forever, reconnecting as needed.
async fn follow_daemon(sockets: Vec<PathBuf>, tx: async_channel::Sender<Link>) {
    let mut connected = None;
    loop {
        match Client::connect_any(&sockets, &client_name()).await {
            Ok(client) => match client.subscribe().await {
                Ok(mut events) => {
                    if tx.send(Link::Connected).await.is_err() {
                        return;
                    }
                    connected = Some(true);
                    loop {
                        match events.next_event().await {
                            Ok(Some(event)) => {
                                if tx.send(Link::Event(event)).await.is_err() {
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
            Err(_) => {
                if connected != Some(false) {
                    connected = Some(false);
                    if tx.send(Link::Disconnected).await.is_err() {
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
