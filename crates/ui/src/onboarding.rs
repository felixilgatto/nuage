//! Adding an account, in at most four screens of one window:
//!
//! 1. the provider;
//! 2. the user's own OAuth client, when the provider needs one and no
//!    account of it exists yet (the client is then reused);
//! 3. signing in in the browser, or the provider's credentials form;
//! 4. the folder and "Start automatically at login" (first account), then
//!    "Start Sync".
//!
//! Nothing provider-specific lives here: the guide, the client ID format
//! and the form come from [`ProviderInfo`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use nuage_core::{Account, AccountAddArgs, AuthKind, Command, CustomClient, ProviderInfo, Reply};

use crate::window::Ui;

/// Starts the setup over the accounts list.
pub fn start(ui: &Rc<Ui>) {
    ui.nav.pop_to_tag("home");
    ui.nav.push(&provider_page(ui));
    ui.present();
}

/// `~/…` for folders in the home directory.
pub fn display_path(path: &Path) -> String {
    let home = glib::home_dir();
    match path.strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

/// A page with a header bar and scrollable content.
fn page(title: &str, content: &impl IsA<gtk::Widget>) -> adw::NavigationPage {
    let clamp = adw::Clamp::builder()
        .maximum_size(520)
        .child(content)
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
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scrolled));
    adw::NavigationPage::new(&view, title)
}

fn vbox() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 24)
}

fn main_button(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .halign(gtk::Align::Center)
        .css_classes(["pill", "suggested-action"])
        .build()
}

fn open_uri(ui: &Ui, url: &str) {
    gtk::UriLauncher::new(url).launch(Some(&ui.window), gio::Cancellable::NONE, |result| {
        if let Err(e) = result {
            tracing::warn!(error = %e, "cannot open the link");
        }
    });
}

// Screen 1: the provider.

fn provider_page(ui: &Rc<Ui>) -> adw::NavigationPage {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Choose Your Cloud"))
        .build();
    for provider in ui.providers() {
        let row = adw::ActionRow::builder()
            .title(&provider.display_name)
            .activatable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(&provider.icon_name));
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let ui = Rc::clone(ui);
        row.connect_activated(move |_| next_after_provider(&ui, &provider));
        group.add(&row);
    }
    let content = vbox();
    content.append(&group);
    page(&gettext("Add Account"), &content)
}

fn next_after_provider(ui: &Rc<Ui>, provider: &ProviderInfo) {
    match provider.auth {
        AuthKind::OAuthBrowser => {
            let has_account = ui
                .accounts()
                .iter()
                .any(|a| a.account.provider_id == provider.id);
            if provider.oauth_client == Some(CustomClient::Required) && !has_account {
                ui.nav.push(&client_page(ui, provider));
            } else {
                ui.nav.push(&sign_in_page(ui, provider, None));
            }
        }
        AuthKind::Credentials => ui.nav.push(&credentials_page(ui, provider)),
        AuthKind::None => create(ui, provider, AccountAddArgs::default(), None),
    }
}

// Screen 2: the user's own OAuth client.

fn client_id_valid(provider: &ProviderInfo, id: &str) -> bool {
    let id = id.trim();
    !id.is_empty()
        && !id.contains(char::is_whitespace)
        && provider
            .client_id_suffix
            .as_deref()
            .is_none_or(|suffix| id.ends_with(suffix) && id.len() > suffix.len())
}

fn client_page(ui: &Rc<Ui>, provider: &ProviderInfo) -> adw::NavigationPage {
    let content = vbox();
    let intro = gtk::Label::builder()
        .label(
            gettext(
                "{provider} needs a sign-in client of your own. Creating it takes about five minutes, once: your other accounts will reuse it.",
            )
            .replace("{provider}", &provider.display_name),
        )
        .wrap(true)
        .xalign(0.0)
        .build();
    content.append(&intro);

    let steps = adw::PreferencesGroup::new();
    for (i, step) in provider.client_guide.iter().enumerate() {
        let row = adw::ActionRow::builder()
            .title(format!("{}. {}", i + 1, gettext(&step.title)))
            .title_lines(3)
            .build();
        let open = gtk::Button::builder()
            .icon_name("adw-external-link-symbolic")
            .tooltip_text(gettext("Open in the Browser"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let ui2 = Rc::clone(ui);
        let url = step.url.clone();
        open.connect_clicked(move |_| open_uri(&ui2, &url));
        row.add_suffix(&open);
        row.set_activatable_widget(Some(&open));
        steps.add(&row);
    }
    content.append(&steps);

    let fields = adw::PreferencesGroup::new();
    let id = adw::EntryRow::builder().title(gettext("Client ID")).build();
    let secret = adw::PasswordEntryRow::builder()
        .title(gettext("Client Secret"))
        .build();
    fields.add(&id);
    fields.add(&secret);
    content.append(&fields);

    let next = main_button(&gettext("Continue"));
    next.set_sensitive(false);
    content.append(&next);

    let update = {
        let (id, secret, next, provider) =
            (id.clone(), secret.clone(), next.clone(), provider.clone());
        move || {
            let ok = client_id_valid(&provider, &id.text())
                && !secret.text().trim().is_empty()
                && !secret.text().contains(char::is_whitespace);
            next.set_sensitive(ok);
            if id.text().is_empty() || client_id_valid(&provider, &id.text()) {
                id.remove_css_class("error");
            } else {
                id.add_css_class("error");
            }
        }
    };
    let u = update.clone();
    id.connect_changed(move |_| u());
    secret.connect_changed(move |_| update());

    let ui2 = Rc::clone(ui);
    let provider2 = provider.clone();
    next.connect_clicked(move |_| {
        let client = (id.text().trim().to_owned(), secret.text().trim().to_owned());
        ui2.nav.push(&sign_in_page(&ui2, &provider2, Some(client)));
    });
    page(&gettext("Your Sign-In Client"), &content)
}

// Screen 3: signing in, or the credentials form.

fn sign_in_page(
    ui: &Rc<Ui>,
    provider: &ProviderInfo,
    client: Option<(String, String)>,
) -> adw::NavigationPage {
    let spinner = gtk::Spinner::builder()
        .spinning(true)
        .width_request(32)
        .height_request(32)
        .halign(gtk::Align::Center)
        .build();
    let label = gtk::Label::builder()
        .label(gettext("Waiting for sign-in in your browser…"))
        .css_classes(["title-3"])
        .wrap(true)
        .justify(gtk::Justification::Center)
        .build();
    let copy = gtk::Button::builder()
        .label(gettext("Copy Sign-In Link"))
        .halign(gtk::Align::Center)
        .css_classes(["flat"])
        .sensitive(false)
        .build();
    let cancel = gtk::Button::builder()
        .label(gettext("Cancel"))
        .halign(gtk::Align::Center)
        .css_classes(["pill"])
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    content.set_valign(gtk::Align::Center);
    content.set_vexpand(true);
    for w in [
        spinner.upcast_ref::<gtk::Widget>(),
        label.upcast_ref(),
        copy.upcast_ref(),
        cancel.upcast_ref(),
    ] {
        content.append(w);
    }
    let nav_page = page(&gettext("Sign In"), &content);

    let session: Rc<RefCell<Option<String>>> = Rc::default();
    let url: Rc<RefCell<Option<String>>> = Rc::default();

    {
        let (url, ui) = (Rc::clone(&url), Rc::clone(ui));
        copy.connect_clicked(move |button| {
            if let Some(url) = url.borrow().as_deref() {
                button.clipboard().set_text(url);
                ui.toast(&gettext("Link copied"));
            }
        });
    }
    {
        let ui = Rc::clone(ui);
        cancel.connect_clicked(move |_| {
            ui.nav.pop();
        });
    }
    // Leaving the page (Cancel or Back) cancels the sign-in. After a
    // success the session is already gone.
    {
        let (ui2, session, this) = (Rc::clone(ui), Rc::clone(&session), nav_page.clone());
        ui.nav.connect_popped(move |_, popped| {
            if popped != &this {
                return;
            }
            if let Some(session) = session.borrow_mut().take() {
                let ui = Rc::clone(&ui2);
                glib::spawn_future_local(async move {
                    let _ = ui.backend.call(Command::SignInCancel { session }).await;
                });
            }
        });
    }

    let args = AccountAddArgs {
        provider: provider.id.clone(),
        client_id: client.as_ref().map(|c| c.0.clone()),
        client_secret: client.as_ref().map(|c| c.1.clone()),
        no_browser: true,
        no_mount: true,
        ..AccountAddArgs::default()
    };
    let ui = Rc::clone(ui);
    let this = nav_page.clone();
    glib::spawn_future_local(async move {
        let started = ui.backend.call(Command::AccountAdd(args)).await;
        let sign_in = match started {
            Ok(Reply::SignIn(s)) => s,
            Ok(Reply::Account(account)) => return show_folder_page(&ui, account),
            Ok(_) => return,
            Err(e) => {
                ui.toast(&e);
                ui.nav.pop();
                return;
            }
        };
        *session.borrow_mut() = Some(sign_in.session.clone());
        *url.borrow_mut() = Some(sign_in.url.clone());
        copy.set_sensitive(true);
        open_uri(&ui, &sign_in.url);

        let done = ui
            .backend
            .call(Command::SignInWait {
                session: sign_in.session,
            })
            .await;
        session.borrow_mut().take();
        match done {
            Ok(Reply::Account(account)) => {
                ui.come_back(
                    &gettext("Signed in to {}").replace("{}", &account.display_name),
                    &gettext("Back to Nuage to choose the folder and start the sync."),
                );
                show_folder_page(&ui, account);
            }
            Ok(_) => {}
            Err(e) => {
                // Back to the client screen, with what to fix.
                if ui.nav.visible_page().as_ref() == Some(&this) {
                    ui.nav.pop();
                }
                ui.toast(&e);
            }
        }
    });
    nav_page
}

fn credentials_page(ui: &Rc<Ui>, provider: &ProviderInfo) -> adw::NavigationPage {
    let content = vbox();
    let group = adw::PreferencesGroup::new();
    let mut entries: Vec<(String, bool, adw::EntryRow)> = Vec::new();
    for field in &provider.credentials {
        let row = if field.secret {
            adw::PasswordEntryRow::builder()
                .title(gettext(&field.label))
                .build()
                .upcast::<adw::EntryRow>()
        } else {
            adw::EntryRow::builder()
                .title(gettext(&field.label))
                .build()
        };
        if field.folder {
            let choose = gtk::Button::builder()
                .icon_name("folder-open-symbolic")
                .tooltip_text(gettext("Choose…"))
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let (ui2, row2) = (Rc::clone(ui), row.clone());
            choose.connect_clicked(move |_| {
                let (ui, row) = (Rc::clone(&ui2), row2.clone());
                glib::spawn_future_local(async move {
                    if let Some(path) = choose_folder(&ui).await {
                        row.set_text(&path.display().to_string());
                    }
                });
            });
            row.add_suffix(&choose);
        }
        group.add(&row);
        entries.push((field.key.clone(), field.required, row));
    }
    content.append(&group);
    let next = main_button(&gettext("Continue"));
    content.append(&next);

    let entries = Rc::new(entries);
    let update = {
        let (entries, next) = (Rc::clone(&entries), next.clone());
        move || {
            next.set_sensitive(
                entries
                    .iter()
                    .all(|(_, required, row)| !required || !row.text().trim().is_empty()),
            );
        }
    };
    update();
    for (_, _, row) in entries.iter() {
        let u = update.clone();
        row.connect_changed(move |_| u());
    }
    let (ui2, provider2) = (Rc::clone(ui), provider.clone());
    next.connect_clicked(move |button| {
        button.set_sensitive(false);
        let params = entries
            .iter()
            .filter(|(_, _, row)| !row.text().trim().is_empty())
            .map(|(key, _, row)| (key.clone(), row.text().trim().to_owned()))
            .collect();
        let args = AccountAddArgs {
            params,
            ..AccountAddArgs::default()
        };
        create(&ui2, &provider2, args, Some(button.clone()));
    });
    page(&provider.display_name, &content)
}

/// Creates an account that needs no browser sign-in.
fn create(ui: &Rc<Ui>, provider: &ProviderInfo, args: AccountAddArgs, button: Option<gtk::Button>) {
    let args = AccountAddArgs {
        provider: provider.id.clone(),
        no_mount: true,
        ..args
    };
    let ui = Rc::clone(ui);
    glib::spawn_future_local(async move {
        match ui.backend.call(Command::AccountAdd(args)).await {
            Ok(Reply::Account(account)) => show_folder_page(&ui, account),
            Ok(_) => {}
            Err(e) => ui.toast(&e),
        }
        if let Some(b) = button {
            b.set_sensitive(true);
        }
    });
}

async fn choose_folder(ui: &Ui) -> Option<PathBuf> {
    let dialog = gtk::FileDialog::builder()
        .title(gettext("Choose a Folder"))
        .modal(true)
        .build();
    dialog
        .select_folder_future(Some(&ui.window))
        .await
        .ok()
        .and_then(|f| f.path())
}

// Screen 4: the folder, then Start Sync.

fn show_folder_page(ui: &Rc<Ui>, account: Account) {
    let first = ui.accounts().is_empty();
    ui.nav.pop_to_tag("home");
    ui.nav.push(&folder_page(ui, account, first));
}

fn folder_page(ui: &Rc<Ui>, account: Account, first: bool) -> adw::NavigationPage {
    let content = vbox();
    let group = adw::PreferencesGroup::builder()
        .description(gettext(
            "Your files appear in this folder. They are downloaded when you open them.",
        ))
        .build();
    let folder = Rc::new(RefCell::new(account.mount_point.clone()));
    let row = adw::ActionRow::builder()
        .title(gettext("Folder"))
        .subtitle(display_path(&account.mount_point))
        .build();
    row.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    let choose = gtk::Button::builder()
        .label(gettext("Choose…"))
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&choose);
    group.add(&row);
    let error = gtk::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .build();

    let autostart = adw::SwitchRow::builder()
        .title(gettext("Start Automatically at Login"))
        .active(true)
        .build();
    if first {
        group.add(&autostart);
    }
    content.append(&group);
    content.append(&error);
    let start = main_button(&gettext("Start Sync"));
    start.set_sensitive(false);
    content.append(&start);

    // Validation: the daemon applies the rules (empty folder, not inside
    // another account's folder…).
    let validate = {
        let (ui, error, start, folder, id) = (
            Rc::clone(ui),
            error.clone(),
            start.clone(),
            Rc::clone(&folder),
            account.id.clone(),
        );
        move || {
            let (ui, error, start, path, id) = (
                Rc::clone(&ui),
                error.clone(),
                start.clone(),
                folder.borrow().clone(),
                id.clone(),
            );
            glib::spawn_future_local(async move {
                let result = ui
                    .backend
                    .call(Command::ValidateMountPoint {
                        account: Some(id),
                        mount_point: path,
                    })
                    .await;
                match result {
                    Ok(_) => {
                        error.set_visible(false);
                        start.set_sensitive(true);
                    }
                    Err(e) => {
                        error.set_label(&e);
                        error.set_visible(true);
                        start.set_sensitive(false);
                    }
                }
            });
        }
    };
    validate();
    {
        let (ui, row, folder, validate) = (
            Rc::clone(ui),
            row.clone(),
            Rc::clone(&folder),
            validate.clone(),
        );
        choose.connect_clicked(move |_| {
            let (ui, row, folder, validate) = (
                Rc::clone(&ui),
                row.clone(),
                Rc::clone(&folder),
                validate.clone(),
            );
            glib::spawn_future_local(async move {
                if let Some(path) = choose_folder(&ui).await {
                    row.set_subtitle(&display_path(&path));
                    *folder.borrow_mut() = path;
                    validate();
                }
            });
        });
    }

    let title = account.display_name.clone();
    let ui2 = Rc::clone(ui);
    start.connect_clicked(move |button| {
        button.set_sensitive(false);
        let (ui, account, path) = (Rc::clone(&ui2), account.clone(), folder.borrow().clone());
        let (button, autostart) = (button.clone(), autostart.clone());
        glib::spawn_future_local(async move {
            let mut account = account;
            if path != account.mount_point {
                match ui
                    .backend
                    .call(Command::AccountSetMountPoint {
                        account: account.id.clone(),
                        mount_point: path,
                    })
                    .await
                {
                    Ok(Reply::Account(a)) => account = a,
                    Ok(_) => {}
                    Err(e) => {
                        ui.toast(&e);
                        button.set_sensitive(true);
                        return;
                    }
                }
            }
            if let Err(e) = ui
                .backend
                .call(Command::Mount {
                    account: account.id.clone(),
                })
                .await
            {
                ui.toast(&e);
                button.set_sensitive(true);
                return;
            }
            if first && autostart.is_active() {
                match ui.backend.request_autostart(true).await {
                    Ok(true) => crate::ini::set("autostart", "true"),
                    Ok(false) => ui.toast(&gettext(
                        "Starting at login was not allowed. You can change it in the system settings.",
                    )),
                    Err(e) => ui.toast(
                        &gettext("Starting at login could not be enabled: {}").replace("{}", &e),
                    ),
                }
            }
            finish(&ui, &account);
        });
    });
    page(&title, &content)
}

/// The account is syncing: tell the user, and leave the window.
fn finish(ui: &Rc<Ui>, account: &Account) {
    let notification = gio::Notification::new(&gettext("Nuage"));
    notification.set_body(Some(
        &gettext("{account} is syncing to {folder}")
            .replace("{account}", &account.display_name)
            .replace("{folder}", &display_path(&account.mount_point)),
    ));
    notification.add_button_with_target_value(
        &gettext("Open Folder"),
        "app.open-folder",
        Some(&account.mount_point.display().to_string().to_variant()),
    );
    ui.app
        .send_notification(Some(&format!("synced-{}", account.id)), &notification);

    ui.nav.pop_to_tag("home");
    let ui2 = Rc::clone(ui);
    glib::spawn_future_local(async move { ui2.reload().await });
    // With a tray icon, the application lives there; otherwise the window
    // is the only way back.
    if ui.tray_available() {
        ui.window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::Capabilities;

    #[test]
    fn client_ids() {
        let p = ProviderInfo {
            id: "gdrive".into(),
            display_name: "Google Drive".into(),
            icon_name: String::new(),
            auth: AuthKind::OAuthBrowser,
            oauth_client: Some(CustomClient::Required),
            credentials: Vec::new(),
            client_guide: Vec::new(),
            client_id_suffix: Some(".apps.googleusercontent.com".into()),
            capabilities: Capabilities::default(),
        };
        assert!(client_id_valid(&p, "123-abc.apps.googleusercontent.com"));
        assert!(client_id_valid(&p, " 123-abc.apps.googleusercontent.com "));
        assert!(!client_id_valid(&p, ".apps.googleusercontent.com"));
        assert!(!client_id_valid(&p, "123 abc.apps.googleusercontent.com"));
        assert!(!client_id_valid(&p, "something-else"));
    }
}
