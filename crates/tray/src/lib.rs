//! System tray icon (StatusNotifierItem + dbusmenu, through `ksni`).
//!
//! GTK4 has no AppIndicator support, so the icon is a pure-Rust D-Bus
//! service. On GNOME it needs the AppIndicator extension (enabled by default
//! on Ubuntu); without it, the main window must be enough.
//!
//! The icon shows the aggregated state; its menu shows each account's state,
//! quota and cache, and the actions of the brief.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use gettextrs::gettext;
use ksni::TrayMethods;
use nuage_core::{StatusReport, SyncState};

pub use ksni::Handle;

/// Something the user asked for from the tray menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    OpenWindow,
    OpenFolder(PathBuf),
    Refresh,
    PauseAll,
    ResumeAll,
    AddAccount,
    Preferences,
    Quit,
}

/// The application's own symbolic icon, shown when everything is up to date.
pub const APP_ICON: &str = "io.github.felixilgatto.Nuage-symbolic";

/// Icon name for an aggregated sync state.
pub fn icon_for_state(state: &SyncState) -> &'static str {
    match state {
        SyncState::UpToDate => APP_ICON,
        SyncState::Syncing { .. } => "emblem-synchronizing-symbolic",
        SyncState::Paused { .. } | SyncState::Stopped => "media-playback-pause-symbolic",
        SyncState::Error { .. } => "dialog-warning-symbolic",
        SyncState::Offline => "network-offline-symbolic",
    }
}

/// One line for a state, as in the main window.
pub fn state_line(state: &SyncState) -> String {
    match state {
        SyncState::UpToDate => gettext("Up to date"),
        SyncState::Syncing { uploads, queued } => {
            gettext("Syncing {} files").replace("{}", &(uploads + queued).to_string())
        }
        SyncState::Paused { .. } => gettext("Paused"),
        SyncState::Error { .. } => gettext("Problem"),
        SyncState::Stopped => gettext("Not mounted"),
        SyncState::Offline => gettext("Offline"),
    }
}

fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// The Nuage tray icon.
pub struct NuageTray {
    status: Option<StatusReport>,
    on_action: Arc<dyn Fn(TrayAction) + Send + Sync>,
    icon_theme_path: String,
}

impl NuageTray {
    /// `on_action` is called from the tray's D-Bus task, not the GTK thread.
    pub fn new(on_action: impl Fn(TrayAction) + Send + Sync + 'static) -> Self {
        Self {
            status: None,
            on_action: Arc::new(on_action),
            icon_theme_path: String::new(),
        }
    }

    /// An extra icon directory (with a `hicolor/` theme inside), for a
    /// source build whose icons are not installed.
    pub fn with_icon_theme_path(mut self, path: impl Into<String>) -> Self {
        self.icon_theme_path = path.into();
        self
    }

    /// `None`: the daemon is not reachable.
    pub fn set_status(&mut self, status: Option<StatusReport>) {
        self.status = status;
    }

    /// Registers the icon on the session bus. Must run inside a tokio runtime.
    ///
    /// Inside a Flatpak the well-known `org.kde.StatusNotifierItem-*` name
    /// cannot be owned, so the item is registered by its unique bus name only.
    pub async fn spawn(self) -> Result<Handle<Self>, ksni::Error> {
        let in_flatpak = std::env::var_os("FLATPAK_ID").is_some()
            || std::path::Path::new("/.flatpak-info").exists();
        self.disable_dbus_name(in_flatpak).spawn().await
    }

    fn state(&self) -> SyncState {
        match &self.status {
            Some(s) => s.state.clone(),
            None => SyncState::Error {
                message: String::new(),
                failed_files: 0,
            },
        }
    }

    fn item(&self, label: String, action: TrayAction) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label,
            activate: Box::new(move |this: &mut Self| (this.on_action)(action.clone())),
            ..Default::default()
        }
        .into()
    }

    fn info(label: String) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label,
            enabled: false,
            ..Default::default()
        }
        .into()
    }
}

impl fmt::Debug for NuageTray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NuageTray")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl ksni::Tray for NuageTray {
    fn id(&self) -> String {
        nuage_core::APP_ID.to_owned()
    }

    fn title(&self) -> String {
        "Nuage".to_owned()
    }

    fn icon_name(&self) -> String {
        icon_for_state(&self.state()).to_owned()
    }

    fn icon_theme_path(&self) -> String {
        self.icon_theme_path.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: format!("Nuage: {}", state_line(&self.state())),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.on_action)(TrayAction::OpenWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut menu = Vec::new();
        let Some(status) = &self.status else {
            menu.push(Self::info(gettext("The Nuage service is not running")));
            menu.push(ksni::MenuItem::Separator);
            menu.push(self.item(gettext("Open Nuage"), TrayAction::OpenWindow));
            menu.push(self.item(gettext("Quit"), TrayAction::Quit));
            return menu;
        };

        menu.push(Self::info(state_line(&status.state)));
        for a in &status.accounts {
            let mut line = format!("{}: {}", a.account.display_name, state_line(&a.state));
            if let Some(q) = a.quota
                && let (Some(used), Some(total)) = (q.used, q.total)
            {
                line.push_str(&format!(" · {} / {}", size(used), size(total)));
            }
            menu.push(Self::info(line));
            menu.push(Self::info(
                gettext("    Cache: {}").replace("{}", &size(a.cache_used)),
            ));
        }
        menu.push(ksni::MenuItem::Separator);
        menu.push(self.item(gettext("Open Nuage"), TrayAction::OpenWindow));

        let mounted: Vec<_> = status.accounts.iter().filter(|a| a.mounted).collect();
        match mounted.as_slice() {
            [] => {}
            [one] => menu.push(self.item(
                gettext("Open Folder"),
                TrayAction::OpenFolder(one.account.mount_point.clone()),
            )),
            many => menu.push(
                ksni::menu::SubMenu {
                    label: gettext("Open Folder"),
                    submenu: many
                        .iter()
                        .map(|a| {
                            self.item(
                                a.account.display_name.clone(),
                                TrayAction::OpenFolder(a.account.mount_point.clone()),
                            )
                        })
                        .collect(),
                    ..Default::default()
                }
                .into(),
            ),
        }
        if !status.accounts.is_empty() {
            menu.push(self.item(gettext("Refresh"), TrayAction::Refresh));
            let all_paused = status
                .accounts
                .iter()
                .all(|a| matches!(a.state, SyncState::Paused { .. }));
            if all_paused {
                menu.push(self.item(gettext("Resume Uploads"), TrayAction::ResumeAll));
            } else {
                menu.push(self.item(gettext("Pause Uploads"), TrayAction::PauseAll));
            }
        }
        if !status.accounts.is_empty() {
            menu.push(self.item(gettext("Preferences"), TrayAction::Preferences));
        }
        menu.push(self.item(gettext("Add Account…"), TrayAction::AddAccount));
        menu.push(ksni::MenuItem::Separator);
        menu.push(self.item(gettext("Quit"), TrayAction::Quit));
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::{Account, AccountId, AccountStatus, Quota};

    fn status(accounts: Vec<AccountStatus>) -> StatusReport {
        StatusReport {
            state: SyncState::aggregate(accounts.iter().map(|a| &a.state)),
            accounts,
            cache_used: 0,
            config_encrypted: true,
        }
    }

    fn account(name: &str, state: SyncState) -> AccountStatus {
        AccountStatus {
            account: Account {
                id: AccountId::new(name),
                provider_id: "local".into(),
                display_name: name.into(),
                remote_name: format!("nuage-{name}"),
                mount_point: format!("/home/me/Cloud/{name}").into(),
                cache_limit: 1 << 30,
            },
            state,
            mounted: true,
            quota: Some(Quota {
                total: Some(2_000_000_000),
                used: Some(500_000_000),
                ..Quota::default()
            }),
            cache_used: 1_500_000,
            offline_used: 0,
            pins_pending: 0,
            pins_conflicts: 0,
        }
    }

    fn labels(tray: &NuageTray) -> Vec<String> {
        use ksni::Tray;
        tray.menu()
            .into_iter()
            .filter_map(|item| match item {
                ksni::MenuItem::Standard(s) => Some(s.label),
                ksni::MenuItem::SubMenu(s) => Some(format!("{} >", s.label)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn icons() {
        assert_eq!(icon_for_state(&SyncState::UpToDate), APP_ICON);
        assert_eq!(APP_ICON, format!("{}-symbolic", nuage_core::APP_ID));
        assert_eq!(
            icon_for_state(&SyncState::Syncing {
                uploads: 1,
                queued: 0
            }),
            "emblem-synchronizing-symbolic"
        );
    }

    #[test]
    fn menu() {
        let mut tray = NuageTray::new(|_| {});
        assert!(labels(&tray).contains(&"The Nuage service is not running".to_owned()));

        tray.set_status(Some(status(vec![account("Drive", SyncState::UpToDate)])));
        let l = labels(&tray);
        assert!(
            l.contains(&"Drive: Up to date · 500.0 MB / 2.0 GB".to_owned()),
            "{l:?}"
        );
        assert!(l.contains(&"Open Folder".to_owned()));
        assert!(l.contains(&"Pause Uploads".to_owned()));
        assert!(l.contains(&"Preferences".to_owned()));

        tray.set_status(Some(status(vec![
            account("A", SyncState::Paused { queued: 0 }),
            account("B", SyncState::Paused { queued: 2 }),
        ])));
        let l = labels(&tray);
        assert!(l.contains(&"Resume Uploads".to_owned()), "{l:?}");
        assert!(l.contains(&"Open Folder >".to_owned()), "{l:?}");
    }
}
