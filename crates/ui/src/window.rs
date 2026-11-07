//! The main window: the accounts, their state and quota, and "+".

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use nuage_core::{AccountId, AccountStatus, Command, ProviderInfo, Reply, StatusReport, SyncState};

use crate::Backend;
use crate::backend::UpdateEvent;

/// Everything the interface shares.
#[derive(Debug)]
pub struct Ui {
    pub app: adw::Application,
    pub backend: Backend,
    pub window: adw::ApplicationWindow,
    pub(crate) nav: adw::NavigationView,
    toasts: adw::ToastOverlay,
    accounts_box: gtk::Box,
    pub(crate) state: RefCell<UiState>,
}

#[derive(Debug, Default)]
pub(crate) struct UiState {
    pub(crate) report: Option<StatusReport>,
    pub(crate) providers: Vec<ProviderInfo>,
    /// The system tray shows the icon: the window can close after setup.
    pub(crate) tray: bool,
    /// Created the first time it is opened.
    pub(crate) browser: Option<Rc<crate::browser::Browser>>,
    /// Where the update of Nuage itself stands, and its toast.
    pub(crate) update: UpdateNotice,
    pub(crate) update_toast: Option<adw::Toast>,
}

/// What the interface last said about an update of Nuage itself.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateNotice {
    #[default]
    None,
    Available,
    Installing,
    Installed,
}

/// ID of the notification about an update of Nuage.
const UPDATE_NOTIFICATION: &str = "update";

/// A short, human state line.
pub fn state_text(state: &SyncState) -> String {
    match state {
        SyncState::UpToDate => gettext("Up to date"),
        SyncState::Syncing { uploads, queued } => {
            let n = uploads + queued;
            if n == 1 {
                gettext("Syncing 1 file")
            } else {
                gettext("Syncing {} files").replace("{}", &n.to_string())
            }
        }
        SyncState::Paused { queued: 0 } => gettext("Paused"),
        SyncState::Paused { queued } => {
            gettext("Paused, {} waiting").replace("{}", &queued.to_string())
        }
        SyncState::Error { message, .. } => message.clone(),
        SyncState::Stopped => gettext("Not mounted"),
        SyncState::Offline => gettext("Offline"),
        SyncState::WaitingForKeyring => {
            gettext("Waiting for the keyring: unlock it to mount the folder")
        }
    }
}

/// Icon of a state, as in the tray.
pub fn state_icon(state: &SyncState) -> &'static str {
    match state {
        SyncState::UpToDate => "emblem-ok-symbolic",
        SyncState::Syncing { .. } => "emblem-synchronizing-symbolic",
        SyncState::Paused { .. } | SyncState::Stopped => "media-playback-pause-symbolic",
        SyncState::Error { .. } => "dialog-warning-symbolic",
        SyncState::Offline => "network-offline-symbolic",
        SyncState::WaitingForKeyring => "dialog-password-symbolic",
    }
}

/// Bytes for people (`1.5 GB`).
pub fn human_size(bytes: u64) -> String {
    glib::format_size(bytes).to_string()
}

/// Opens a folder in the file manager.
pub fn open_folder(window: &impl IsA<gtk::Window>, path: &std::path::Path) {
    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(path)));
    launcher.launch(Some(window), gio::Cancellable::NONE, |result| {
        if let Err(e) = result {
            tracing::warn!(error = %e, "cannot open the folder");
        }
    });
}

impl Ui {
    pub fn new(app: &adw::Application, backend: Backend) -> Rc<Self> {
        let nav = adw::NavigationView::new();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&nav));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Nuage")
            .default_width(560)
            .default_height(620)
            .content(&toasts)
            .build();
        // Closing the window does not stop anything: it only hides it.
        window.set_hide_on_close(true);

        let accounts_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let ui = Rc::new(Self {
            app: app.clone(),
            backend,
            window,
            nav,
            toasts,
            accounts_box,
            state: RefCell::new(UiState::default()),
        });
        ui.nav.add(&ui.home_page());
        ui
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// A toast with one button running `action` (an `app.` action).
    pub fn toast_with_action(&self, text: &str, button: &str, action: &str, target: glib::Variant) {
        let toast = adw::Toast::builder()
            .title(text)
            .button_label(button)
            .action_name(action)
            .action_target(&target)
            .timeout(10)
            .build();
        self.toasts.add_toast(toast);
    }

    /// Tells the user about an update of Nuage itself, once per step.
    pub fn update_event(&self, event: UpdateEvent) {
        let notice = self.state.borrow().update;
        match event {
            UpdateEvent::Available if notice == UpdateNotice::None => self.update_available(),
            UpdateEvent::Installed if notice != UpdateNotice::Installed => self.update_installed(),
            UpdateEvent::Failed(e) => self.update_failed(&e),
            _ => {}
        }
    }

    /// Installs the available update (button "Update").
    pub fn install_update(&self) {
        if self.state.borrow().update == UpdateNotice::Installing {
            return;
        }
        self.show_update_toast(
            adw::Toast::builder()
                .title(gettext("Updating Nuage…"))
                .timeout(0)
                .build(),
        );
        self.app.withdraw_notification(UPDATE_NOTIFICATION);
        self.state.borrow_mut().update = UpdateNotice::Installing;
        self.backend.install_update();
    }

    /// Replaces the toast about the update, if any, with `toast`.
    fn show_update_toast(&self, toast: adw::Toast) {
        if let Some(old) = self.state.borrow_mut().update_toast.replace(toast.clone()) {
            old.dismiss();
        }
        self.toasts.add_toast(toast);
    }

    /// A newer version is in the repository: offer to install it (a toast
    /// that stays, and a notification, since the window is often closed).
    fn update_available(&self) {
        let text = gettext("A new version of Nuage is available");
        self.show_update_toast(
            adw::Toast::builder()
                .title(&text)
                .button_label(gettext("Update"))
                .action_name("app.install-update")
                .timeout(0)
                .build(),
        );
        let notification = gio::Notification::new(&text);
        notification.set_body(Some(&gettext(
            "Update now, or later from Software. Your folders stay mounted while it downloads.",
        )));
        notification.set_default_action("app.open-window");
        notification.add_button(&gettext("Update"), "app.install-update");
        self.app
            .send_notification(Some(UPDATE_NOTIFICATION), &notification);
        self.state.borrow_mut().update = UpdateNotice::Available;
    }

    /// A newer version is installed: offer to restart (a toast that stays,
    /// and a notification, since the window is often closed).
    fn update_installed(&self) {
        let text = gettext("A new version of Nuage is installed");
        self.show_update_toast(
            adw::Toast::builder()
                .title(&text)
                .button_label(gettext("Restart"))
                .action_name("app.restart")
                .timeout(0)
                .build(),
        );
        let notification = gio::Notification::new(&text);
        notification.set_body(Some(&gettext(
            "Restart Nuage to use it. Your folders are unmounted for a moment.",
        )));
        notification.set_default_action("app.open-window");
        notification.add_button(&gettext("Restart"), "app.restart");
        self.app
            .send_notification(Some(UPDATE_NOTIFICATION), &notification);
        self.state.borrow_mut().update = UpdateNotice::Installed;
    }

    /// Installing the update failed: say so, and where else to update. The
    /// portal's message is technical: it goes to the log only.
    fn update_failed(&self, error: &str) {
        tracing::warn!(error, "cannot install the update");
        let text = gettext("Nuage could not update itself. Update it from Software instead.");
        self.show_update_toast(adw::Toast::builder().title(&text).timeout(0).build());
        let notification = gio::Notification::new(&gettext("Nuage could not update itself"));
        notification.set_body(Some(&gettext("Update it from Software instead.")));
        notification.set_default_action("app.open-window");
        self.app
            .send_notification(Some(UPDATE_NOTIFICATION), &notification);
        // Offered again if the monitor reports the update again.
        self.state.borrow_mut().update = UpdateNotice::None;
    }

    pub fn set_tray_available(&self, available: bool) {
        self.state.borrow_mut().tray = available;
    }

    pub fn tray_available(&self) -> bool {
        self.state.borrow().tray
    }

    /// Shows the window, at the accounts list.
    pub fn present(&self) {
        self.window.present();
    }

    /// Brings the window back after the user left it for the browser.
    ///
    /// On Wayland an application cannot raise itself without an activation
    /// token, which only comes from the user (e.g. clicking a notification):
    /// `present` may only flash it. So a notification also says what to do
    /// next; clicking it raises the window, and it is withdrawn as soon as
    /// the window is active again.
    pub fn come_back(&self, title: &str, body: &str) {
        const ID: &str = "come-back";
        self.window.present();
        if self.window.is_active() {
            return;
        }
        let notification = gio::Notification::new(title);
        notification.set_body(Some(body));
        notification.set_priority(gio::NotificationPriority::High);
        notification.set_default_action("app.open-window");
        self.app.send_notification(Some(ID), &notification);
        let app = self.app.clone();
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let h = Rc::clone(&handler);
        let id = self.window.connect_is_active_notify(move |window| {
            if window.is_active() {
                app.withdraw_notification(ID);
                if let Some(id) = h.borrow_mut().take() {
                    window.disconnect(id);
                }
            }
        });
        *handler.borrow_mut() = Some(id);
    }

    fn home_page(self: &Rc<Self>) -> adw::NavigationPage {
        let header = adw::HeaderBar::new();
        let add = gtk::Button::from_icon_name("list-add-symbolic");
        add.set_tooltip_text(Some(&gettext("Add Account")));
        add.set_action_name(Some("app.add-account"));
        header.pack_start(&add);

        let menu = gio::Menu::new();
        menu.append(Some(&gettext("_Preferences")), Some("app.preferences"));
        menu.append(Some(&gettext("_About Nuage")), Some("app.about"));
        menu.append(Some(&gettext("_Quit")), Some("app.quit-all"));
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .primary(true)
            .tooltip_text(gettext("Main Menu"))
            .build();
        header.pack_end(&menu_button);

        let clamp = adw::Clamp::builder()
            .maximum_size(560)
            .child(&self.accounts_box)
            .margin_top(18)
            .margin_bottom(18)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .vexpand(true)
            .build();

        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&scrolled));
        adw::NavigationPage::builder()
            .title("Nuage")
            .tag("home")
            .child(&view)
            .build()
    }

    /// Reloads the providers and the status from the daemon.
    pub async fn reload(self: &Rc<Self>) {
        if let Ok(Reply::Providers(p)) = self.backend.call(Command::ListProviders).await {
            self.state.borrow_mut().providers = p;
        }
        match self.backend.call(Command::Status).await {
            Ok(Reply::Status(report)) => self.show_status(report),
            Ok(_) => {}
            Err(e) => self.show_disconnected(&e),
        }
    }

    /// The last status report shown.
    pub fn status(&self) -> Option<StatusReport> {
        self.state.borrow().report.clone()
    }

    pub fn providers(&self) -> Vec<ProviderInfo> {
        self.state.borrow().providers.clone()
    }

    pub fn accounts(&self) -> Vec<AccountStatus> {
        self.state
            .borrow()
            .report
            .as_ref()
            .map(|r| r.accounts.clone())
            .unwrap_or_default()
    }

    /// The daemon is not reachable.
    pub fn show_disconnected(&self, message: &str) {
        clear(&self.accounts_box);
        let page = adw::StatusPage::builder()
            .icon_name("network-offline-symbolic")
            .title(gettext("The Nuage Service Is Not Running"))
            .description(message)
            .vexpand(true)
            .build();
        self.accounts_box.append(&page);
    }

    /// Shows a new status report.
    pub fn show_status(self: &Rc<Self>, report: StatusReport) {
        clear(&self.accounts_box);
        if report.accounts.is_empty() {
            let add = gtk::Button::builder()
                .label(gettext("Add Account"))
                .action_name("app.add-account")
                .halign(gtk::Align::Center)
                .css_classes(["pill", "suggested-action"])
                .build();
            let page = adw::StatusPage::builder()
                .icon_name(nuage_core::APP_ID)
                .title(gettext("No Accounts"))
                .description(gettext(
                    "Connect a cloud storage account to see it as a folder.",
                ))
                .child(&add)
                .vexpand(true)
                .build();
            self.accounts_box.append(&page);
        } else {
            let group = adw::PreferencesGroup::new();
            for status in &report.accounts {
                group.add(&self.account_row(status));
            }
            self.accounts_box.append(&group);
        }
        self.state.borrow_mut().report = Some(report);
        let browser = self.state.borrow().browser.clone();
        if let Some(browser) = browser {
            browser.update_accounts();
        }
    }

    /// Opens the file browser, at `account` if given.
    pub fn open_browser(self: &Rc<Self>, account: Option<&AccountId>) {
        let existing = self.state.borrow().browser.clone();
        let browser = existing.unwrap_or_else(|| {
            let b = crate::browser::Browser::new(self);
            self.state.borrow_mut().browser = Some(Rc::clone(&b));
            b
        });
        browser.present(account);
    }

    fn account_row(self: &Rc<Self>, status: &AccountStatus) -> adw::ActionRow {
        let account = &status.account;
        let icon = self
            .providers()
            .iter()
            .find(|p| p.id == account.provider_id)
            .map(|p| p.icon_name.clone())
            .unwrap_or_else(|| "folder-remote-symbolic".to_owned());

        let mut subtitle = state_text(&status.state);
        if let Some(q) = &status.quota
            && let (Some(used), Some(total)) = (q.used, q.total)
        {
            subtitle.push_str(" · ");
            subtitle.push_str(
                &gettext("{used} of {total} used")
                    .replace("{used}", &human_size(used))
                    .replace("{total}", &human_size(total)),
            );
        }
        let row = adw::ActionRow::builder()
            .title(&account.display_name)
            .subtitle(glib::markup_escape_text(&subtitle))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(&icon));
        let state_image = gtk::Image::from_icon_name(state_icon(&status.state));
        state_image.set_tooltip_text(Some(&state_text(&status.state)));
        row.add_suffix(&state_image);

        if let Some(percent) = status.quota.and_then(|q| q.used_percent()) {
            let bar = gtk::LevelBar::builder()
                .min_value(0.0)
                .max_value(100.0)
                .value(percent)
                .width_request(64)
                .valign(gtk::Align::Center)
                .build();
            row.add_suffix(&bar);
        }

        if status.mounted {
            let open = gtk::Button::builder()
                .icon_name("folder-open-symbolic")
                .tooltip_text(gettext("Open Folder"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let window = self.window.clone();
            let path: PathBuf = account.mount_point.clone();
            open.connect_clicked(move |_| open_folder(&window, &path));
            row.add_suffix(&open);
        } else {
            let mount = gtk::Button::builder()
                .label(gettext("Mount"))
                .valign(gtk::Align::Center)
                .build();
            let ui = Rc::clone(self);
            let id = account.id.clone();
            mount.connect_clicked(move |_| {
                let ui = Rc::clone(&ui);
                let id = id.clone();
                glib::spawn_future_local(async move {
                    if let Err(e) = ui.backend.call(Command::Mount { account: id }).await {
                        ui.toast(&e);
                    }
                    ui.reload().await;
                });
            });
            row.add_suffix(&mount);
        }
        let settings = gtk::Button::builder()
            .icon_name("emblem-system-symbolic")
            .tooltip_text(gettext("Account Settings"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .action_name("app.account-preferences")
            .action_target(&account.id.as_str().to_variant())
            .build();
        row.add_suffix(&settings);
        // The browser opens from an account.
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        row.set_tooltip_text(Some(&gettext("Browse Files")));
        let ui = Rc::clone(self);
        let id = account.id.clone();
        row.connect_activated(move |_| ui.open_browser(Some(&id)));
        row
    }
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
