//! System tray icon (StatusNotifierItem + dbusmenu, through `ksni`).
//!
//! GTK4 has no AppIndicator support, so the icon is a pure-Rust D-Bus
//! service. On GNOME it needs the AppIndicator extension (enabled by default
//! on Ubuntu); without it, the main window must be enough.
//!
//! **Skeleton**: the icon shows the aggregated state and a minimal menu. The
//! full menu (quota, pause, refresh, preferences…) comes in milestone 6.

use std::fmt;
use std::sync::Arc;

use ksni::TrayMethods;
use nuage_core::SyncState;

pub use ksni::Handle;

/// Something the user asked for from the tray menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenWindow,
    Quit,
}

/// Icon name for an aggregated sync state.
pub fn icon_for_state(state: &SyncState) -> &'static str {
    match state {
        SyncState::UpToDate => "emblem-ok-symbolic",
        SyncState::Syncing { .. } => "emblem-synchronizing-symbolic",
        SyncState::Paused { .. } | SyncState::Stopped => "media-playback-pause-symbolic",
        SyncState::Error { .. } => "dialog-warning-symbolic",
        SyncState::Offline => "network-offline-symbolic",
    }
}

/// The Nuage tray icon.
pub struct NuageTray {
    state: SyncState,
    on_action: Arc<dyn Fn(TrayAction) + Send + Sync>,
}

impl NuageTray {
    /// `on_action` is called from the tray's D-Bus task, not the GTK thread.
    pub fn new(on_action: impl Fn(TrayAction) + Send + Sync + 'static) -> Self {
        Self {
            state: SyncState::UpToDate,
            on_action: Arc::new(on_action),
        }
    }

    pub fn set_state(&mut self, state: SyncState) {
        self.state = state;
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
}

impl fmt::Debug for NuageTray {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NuageTray")
            .field("state", &self.state)
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
        icon_for_state(&self.state).to_owned()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        (self.on_action)(TrayAction::OpenWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Open Nuage".into(),
                activate: Box::new(|this: &mut Self| (this.on_action)(TrayAction::OpenWindow)),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| (this.on_action)(TrayAction::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons() {
        assert_eq!(icon_for_state(&SyncState::UpToDate), "emblem-ok-symbolic");
        assert_eq!(
            icon_for_state(&SyncState::Syncing {
                uploads: 1,
                queued: 0
            }),
            "emblem-synchronizing-symbolic"
        );
    }
}
