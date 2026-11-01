//! Preferences (`AdwPreferencesDialog`).
//!
//! - General: start at login, what "Quit" does with pending uploads, the
//!   sign-in clients of providers that use the user's own.
//! - One page per account: name, folder, sign-in, cache limit and age,
//!   exclusions, removal.
//!
//! Every change goes to the daemon at once; nothing is applied on close.

use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::glib;

use nuage_core::{
    AccountStatus, AuthKind, Command, CustomClient, DEFAULT_PATTERNS, Error, Exclusion,
    ExclusionRule, ProviderInfo, Reply,
};

use crate::onboarding::display_path;
use crate::window::Ui;

/// Values of the daemon's `quit-with-pending` setting, in display order.
const QUIT_CHOICES: [&str; 3] = ["finish-in-background", "ask", "quit-now"];

/// Choices of "Warn when offline items exceed": `offline-warning`.
const WARNING_CHOICES: [&str; 5] = ["off", "1G", "5G", "10G", "50G"];

/// Choices of "Check the connection every": `offline-check-interval`.
const CHECK_CHOICES: [&str; 4] = ["30s", "1m", "5m", "15m"];

fn check_label(value: &str) -> String {
    match value {
        "30s" => gettext("30 seconds"),
        "1m" => gettext("1 minute"),
        "5m" => gettext("5 minutes"),
        "15m" => gettext("15 minutes"),
        other => other.to_owned(),
    }
}

/// Choices of "Keep cached files for": `cache-max-age` values.
const AGE_CHOICES: [&str; 6] = ["default", "1d", "1w", "30d", "90d", "365d"];

fn age_label(value: &str) -> String {
    match value {
        "default" => gettext("Default (30 days)"),
        "1d" => gettext("1 day"),
        "1w" => gettext("1 week"),
        "30d" => gettext("30 days"),
        "90d" => gettext("90 days"),
        "365d" => gettext("1 year"),
        other => other.to_owned(),
    }
}

/// `"20.0 GB"`-like setting values back to whole gigabytes.
fn gigabytes(value: &str) -> Option<f64> {
    let v = value.trim();
    let (num, unit) = v.split_at(v.find(|c: char| c.is_alphabetic()).unwrap_or(v.len()));
    let num: f64 = num.trim().parse().ok()?;
    let factor = match unit.trim().to_ascii_uppercase().as_str() {
        "T" | "TB" | "TIB" => 1024.0,
        "G" | "GB" | "GIB" => 1.0,
        "M" | "MB" | "MIB" => 1.0 / 1024.0,
        "" => 1.0 / f64::from(1u32 << 30),
        _ => return None,
    };
    Some((num * factor).round())
}

/// Opens the preferences over `parent`, at `account`'s page if given.
pub fn show(ui: &Rc<Ui>, parent: &impl IsA<gtk::Widget>, account: Option<&nuage_core::AccountId>) {
    let dialog = adw::PreferencesDialog::builder()
        .search_enabled(false)
        .build();
    dialog.add(&general_page(ui, &dialog));
    let providers = ui.providers();
    for status in ui.accounts() {
        let provider = providers
            .iter()
            .find(|p| p.id == status.account.provider_id)
            .cloned();
        let page = account_page(ui, &dialog, &status, provider.as_ref());
        dialog.add(&page);
        if account == Some(&status.account.id) {
            dialog.set_visible_page(&page);
        }
    }
    dialog.present(Some(parent));
}

/// Sends a setting; errors show as toasts on the dialog.
fn set(ui: &Rc<Ui>, dialog: &adw::PreferencesDialog, key: String, value: String) {
    let (ui, dialog) = (Rc::clone(ui), dialog.clone());
    glib::spawn_future_local(async move {
        if let Err(e) = ui.backend.call(Command::ConfigSet { key, value }).await {
            dialog.add_toast(adw::Toast::new(&e));
        }
    });
}

async fn alert(parent: &adw::PreferencesDialog, heading: &str, body: &str) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(body)
        .build();
    dialog.add_response("ok", &gettext("_OK"));
    dialog.choose_future(Some(parent)).await;
}

async fn get(ui: &Ui, key: &str) -> Option<String> {
    match ui
        .backend
        .call(Command::ConfigGet {
            key: Some(key.to_owned()),
        })
        .await
    {
        Ok(Reply::Config(entries)) => entries.into_iter().next().map(|e| e.value),
        _ => None,
    }
}

// General

fn general_page(ui: &Rc<Ui>, dialog: &adw::PreferencesDialog) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title(gettext("General"))
        .icon_name("preferences-system-symbolic")
        .build();

    let startup = adw::PreferencesGroup::new();
    let autostart = adw::SwitchRow::builder()
        .title(gettext("Start Automatically at Login"))
        .subtitle(gettext("Keep your folders mounted and synced"))
        .active(crate::ini::get("autostart").as_deref() == Some("true"))
        .build();
    {
        let (ui, dialog) = (Rc::clone(ui), dialog.clone());
        // Set while the switch is put back after a refusal.
        let reverting = Rc::new(std::cell::Cell::new(false));
        autostart.connect_active_notify(move |row| {
            if reverting.get() {
                return;
            }
            let want = row.is_active();
            let (ui, dialog, row) = (Rc::clone(&ui), dialog.clone(), row.clone());
            let reverting = Rc::clone(&reverting);
            let revert = move || {
                reverting.set(true);
                row.set_active(!want);
                reverting.set(false);
            };
            glib::spawn_future_local(async move {
                match ui.backend.request_autostart(want).await {
                    Ok(granted) if granted == want => {
                        crate::ini::set("autostart", if want { "true" } else { "false" });
                    }
                    // Long messages: a dialog, not a toast that cuts them.
                    Ok(_) => {
                        revert();
                        alert(
                            &dialog,
                            &gettext("Starting at Login Was Not Allowed"),
                            &gettext("You can allow it in Settings → Apps → Nuage."),
                        )
                        .await;
                    }
                    Err(e) => {
                        revert();
                        alert(&dialog, &gettext("Cannot Start at Login"), &e).await;
                    }
                }
            });
        });
    }
    startup.add(&autostart);
    page.add(&startup);

    let quit_group = adw::PreferencesGroup::new();
    let labels = [
        gettext("Finish in background"),
        gettext("Ask"),
        gettext("Quit now"),
    ];
    let quit = adw::ComboRow::builder()
        .title(gettext("When Quitting With Pending Uploads"))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    quit_group.add(&quit);
    let labels: Vec<String> = CHECK_CHOICES.iter().map(|v| check_label(v)).collect();
    let check = adw::ComboRow::builder()
        .title(gettext("Check the Connection Every"))
        .subtitle(gettext("Items kept offline are synced at the same pace"))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    quit_group.add(&check);
    let warning_labels = [
        gettext("Never"),
        gettext("1 GB"),
        gettext("5 GB"),
        gettext("10 GB"),
        gettext("50 GB"),
    ];
    let warning = adw::ComboRow::builder()
        .title(gettext("Warn When Offline Items Exceed"))
        .subtitle(gettext("Per account"))
        .model(&gtk::StringList::new(
            &warning_labels
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ))
        .build();
    quit_group.add(&warning);
    {
        let (ui, dialog, warning) = (Rc::clone(ui), dialog.clone(), warning.clone());
        glib::spawn_future_local(async move {
            if let Some(value) = get(&ui, "offline-warning").await
                && let Some(i) = WARNING_CHOICES.iter().position(|c| *c == value)
            {
                warning.set_selected(i as u32);
            }
            warning.connect_selected_notify(move |row| {
                if let Some(value) = WARNING_CHOICES.get(row.selected() as usize) {
                    set(&ui, &dialog, "offline-warning".into(), (*value).into());
                }
            });
        });
    }
    {
        let (ui, dialog, check) = (Rc::clone(ui), dialog.clone(), check.clone());
        glib::spawn_future_local(async move {
            if let Some(value) = get(&ui, "offline-check-interval").await
                && let Some(i) = CHECK_CHOICES.iter().position(|c| *c == value)
            {
                check.set_selected(i as u32);
            }
            check.connect_selected_notify(move |row| {
                if let Some(value) = CHECK_CHOICES.get(row.selected() as usize) {
                    set(
                        &ui,
                        &dialog,
                        "offline-check-interval".into(),
                        (*value).into(),
                    );
                }
            });
        });
    }
    page.add(&quit_group);
    {
        let (ui, dialog, quit) = (Rc::clone(ui), dialog.clone(), quit.clone());
        glib::spawn_future_local(async move {
            if let Some(value) = get(&ui, "quit-with-pending").await
                && let Some(i) = QUIT_CHOICES.iter().position(|c| *c == value)
            {
                quit.set_selected(i as u32);
            }
            quit.connect_selected_notify(move |row| {
                if let Some(value) = QUIT_CHOICES.get(row.selected() as usize) {
                    set(&ui, &dialog, "quit-with-pending".into(), (*value).into());
                }
            });
        });
    }

    // Sign-in clients of the providers that use the user's own.
    let clients = adw::PreferencesGroup::builder()
        .title(gettext("Sign-In Clients"))
        .description(gettext(
            "The OAuth clients you created. Changing one signs its accounts out until you sign in again.",
        ))
        .build();
    let accounts = ui.accounts();
    for provider in ui.providers() {
        let own = matches!(
            provider.oauth_client,
            Some(CustomClient::Required | CustomClient::Optional)
        );
        if provider.auth != AuthKind::OAuthBrowser
            || !own
            || !accounts
                .iter()
                .any(|a| a.account.provider_id == provider.id)
        {
            continue;
        }
        clients.add(&client_row(ui, dialog, &provider));
    }
    if clients.first_child().is_some() && accounts_use_clients(ui) {
        page.add(&clients);
    }
    page
}

fn accounts_use_clients(ui: &Ui) -> bool {
    let providers = ui.providers();
    ui.accounts().iter().any(|a| {
        providers.iter().any(|p| {
            p.id == a.account.provider_id
                && matches!(
                    p.oauth_client,
                    Some(CustomClient::Required | CustomClient::Optional)
                )
        })
    })
}

fn client_row(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    provider: &ProviderInfo,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(&provider.display_name)
        .subtitle(gettext("Loading…"))
        .subtitle_lines(1)
        .build();
    row.add_prefix(&gtk::Image::from_icon_name(&provider.icon_name));
    let change = gtk::Button::builder()
        .label(gettext("Change…"))
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&change);
    {
        let (ui, row, id) = (Rc::clone(ui), row.clone(), provider.id.clone());
        glib::spawn_future_local(async move {
            let subtitle = match ui
                .backend
                .call(Command::OAuthClientGet { provider: id })
                .await
            {
                Ok(Reply::OAuthClient(info)) => info
                    .client_id
                    .unwrap_or_else(|| gettext("rclone's shared client")),
                Ok(_) => String::new(),
                Err(e) => e,
            };
            row.set_subtitle(&glib::markup_escape_text(&subtitle));
        });
    }
    {
        let (ui, dialog, provider, row) =
            (Rc::clone(ui), dialog.clone(), provider.clone(), row.clone());
        change.connect_clicked(move |_| {
            let (ui, dialog, provider, row) = (
                Rc::clone(&ui),
                dialog.clone(),
                provider.clone(),
                row.clone(),
            );
            glib::spawn_future_local(async move {
                change_client(&ui, &dialog, &provider, &row).await;
            });
        });
    }
    row
}

async fn change_client(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    provider: &ProviderInfo,
    row: &adw::ActionRow,
) {
    let fields = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    let id = adw::EntryRow::builder().title(gettext("Client ID")).build();
    let secret = adw::PasswordEntryRow::builder()
        .title(gettext("Client Secret"))
        .build();
    fields.append(&id);
    fields.append(&secret);
    let ask = adw::AlertDialog::builder()
        .heading(gettext("Change the Sign-In Client"))
        .body(
            gettext(
                "Paste the ID and secret of the client you created for {}. Its accounts then ask you to sign in again.",
            )
            .replace("{}", &provider.display_name),
        )
        .extra_child(&fields)
        .build();
    ask.add_response("cancel", &gettext("_Cancel"));
    ask.add_response("change", &gettext("C_hange"));
    ask.set_response_appearance("change", adw::ResponseAppearance::Suggested);
    ask.set_response_enabled("change", false);
    ask.set_close_response("cancel");
    {
        let (ask2, id2, secret2) = (ask.clone(), id.clone(), secret.clone());
        let suffix = provider.client_id_suffix.clone();
        let check = move || {
            let i = id2.text();
            let i = i.trim();
            let ok_id = !i.is_empty()
                && !i.contains(char::is_whitespace)
                && suffix
                    .as_deref()
                    .is_none_or(|s| i.len() > s.len() && i.ends_with(s));
            ask2.set_response_enabled("change", ok_id && !secret2.text().trim().is_empty());
        };
        let c = check.clone();
        id.connect_changed(move |_| c());
        secret.connect_changed(move |_| check());
    }
    if ask.choose_future(Some(dialog)).await != "change" {
        return;
    }
    let (client_id, client_secret) = (id.text().trim().to_owned(), secret.text().trim().to_owned());
    match ui
        .backend
        .call(Command::OAuthClientSet {
            provider: provider.id.clone(),
            client_id: client_id.clone(),
            client_secret,
        })
        .await
    {
        Ok(_) => {
            row.set_subtitle(&glib::markup_escape_text(&client_id));
            // Sign in again, account by account.
            let accounts: Vec<AccountStatus> = ui
                .accounts()
                .into_iter()
                .filter(|a| a.account.provider_id == provider.id)
                .collect();
            for a in accounts {
                if !crate::reconnect::reconnect(ui, &a.account.id, dialog).await {
                    break;
                }
            }
        }
        Err(e) => dialog.add_toast(adw::Toast::new(&e)),
    }
}

// Accounts

fn account_page(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    status: &AccountStatus,
    provider: Option<&ProviderInfo>,
) -> adw::PreferencesPage {
    let account = &status.account;
    let id = account.id.clone();
    let page = adw::PreferencesPage::builder()
        .title(&account.display_name)
        .icon_name(
            provider
                .map(|p| p.icon_name.as_str())
                .unwrap_or("folder-remote-symbolic"),
        )
        .build();

    // Name, folder, sign-in.
    let main = adw::PreferencesGroup::new();
    let name = adw::EntryRow::builder()
        .title(gettext("Name"))
        .text(&account.display_name)
        .show_apply_button(true)
        .build();
    {
        let (ui, dialog, id, page) = (Rc::clone(ui), dialog.clone(), id.clone(), page.clone());
        name.connect_apply(move |row| {
            let new = row.text().trim().to_owned();
            let (ui, dialog, id, page) = (Rc::clone(&ui), dialog.clone(), id.clone(), page.clone());
            glib::spawn_future_local(async move {
                match ui
                    .backend
                    .call(Command::AccountRename {
                        account: id,
                        name: new.clone(),
                    })
                    .await
                {
                    Ok(_) => {
                        page.set_title(&new);
                        ui.reload().await;
                    }
                    Err(e) => dialog.add_toast(adw::Toast::new(&e)),
                }
            });
        });
    }
    main.add(&name);

    let folder = adw::ActionRow::builder()
        .title(gettext("Folder"))
        .subtitle(glib::markup_escape_text(&display_path(
            &account.mount_point,
        )))
        .build();
    let choose = gtk::Button::builder()
        .label(gettext("Choose…"))
        .valign(gtk::Align::Center)
        .build();
    folder.add_suffix(&choose);
    {
        let (ui, dialog, id, folder) = (Rc::clone(ui), dialog.clone(), id.clone(), folder.clone());
        choose.connect_clicked(move |_| {
            let (ui, dialog, id, folder) =
                (Rc::clone(&ui), dialog.clone(), id.clone(), folder.clone());
            glib::spawn_future_local(async move {
                move_folder(&ui, &dialog, &id, &folder).await;
            });
        });
    }
    main.add(&folder);

    if provider.is_some_and(|p| p.auth == AuthKind::OAuthBrowser) {
        let sign_in = adw::ActionRow::builder()
            .title(gettext("Sign-In"))
            .subtitle(match &status.state {
                nuage_core::SyncState::Error { message, .. } => {
                    glib::markup_escape_text(message).to_string()
                }
                _ => gettext("Connected"),
            })
            .build();
        let reconnect = gtk::Button::builder()
            .label(gettext("Sign In Again…"))
            .valign(gtk::Align::Center)
            .build();
        {
            let (ui, dialog, id, sign_in) =
                (Rc::clone(ui), dialog.clone(), id.clone(), sign_in.clone());
            reconnect.connect_clicked(move |_| {
                let (ui, dialog, id, sign_in) =
                    (Rc::clone(&ui), dialog.clone(), id.clone(), sign_in.clone());
                glib::spawn_future_local(async move {
                    if crate::reconnect::reconnect(&ui, &id, &dialog).await {
                        sign_in.set_subtitle(&gettext("Connected"));
                    }
                });
            });
        }
        sign_in.add_suffix(&reconnect);
        main.add(&sign_in);
    }
    page.add(&main);

    page.add(&cache_group(ui, dialog, status));
    page.add(&offline_group(ui, dialog, status));
    page.add(&exclusions_group(ui, dialog, status));

    let danger = adw::PreferencesGroup::new();
    let remove = gtk::Button::builder()
        .label(gettext("Remove Account…"))
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action", "pill"])
        .build();
    {
        let (ui, dialog, status) = (Rc::clone(ui), dialog.clone(), status.clone());
        remove.connect_clicked(move |_| {
            let (ui, dialog, status) = (Rc::clone(&ui), dialog.clone(), status.clone());
            glib::spawn_future_local(async move { remove_account(&ui, &dialog, &status).await });
        });
    }
    danger.add(&remove);
    page.add(&danger);
    page
}

async fn move_folder(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    id: &nuage_core::AccountId,
    row: &adw::ActionRow,
) {
    let chooser = gtk::FileDialog::builder()
        .title(gettext("Choose a Folder"))
        .modal(true)
        .build();
    let window = dialog.root().and_downcast::<gtk::Window>();
    let Ok(file) = chooser.select_folder_future(window.as_ref()).await else {
        return;
    };
    let Some(path) = file.path() else { return };
    let answer = ui
        .backend
        .call(Command::AccountSetMountPoint {
            account: id.clone(),
            mount_point: path.clone(),
        })
        .await;
    match answer {
        Ok(_) => {
            row.set_subtitle(&glib::markup_escape_text(&display_path(&path)));
            dialog.add_toast(adw::Toast::new(&gettext("Moved")));
            ui.reload().await;
        }
        Err(e) => dialog.add_toast(adw::Toast::new(&e)),
    }
}

fn cache_group(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    status: &AccountStatus,
) -> adw::PreferencesGroup {
    let id = status.account.id.as_str().to_owned();
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Cache"))
        .description(
            gettext("Files you opened are kept on this computer, up to a limit. In use: {}.")
                .replace("{}", &crate::window::human_size(status.cache_used)),
        )
        .build();
    let limit = adw::SpinRow::builder()
        .title(gettext("Cache Limit"))
        .subtitle(gettext("In gigabytes"))
        .adjustment(&gtk::Adjustment::new(
            (status.account.cache_limit as f64 / f64::from(1u32 << 30)).round(),
            1.0,
            4096.0,
            1.0,
            10.0,
            0.0,
        ))
        .build();
    group.add(&limit);
    let labels: Vec<String> = AGE_CHOICES.iter().map(|v| age_label(v)).collect();
    let age = adw::ComboRow::builder()
        .title(gettext("Keep Cached Files For"))
        .model(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    group.add(&age);

    let (ui, dialog) = (Rc::clone(ui), dialog.clone());
    glib::spawn_future_local(async move {
        if let Some(v) = get(&ui, &format!("{id}:cache-limit")).await
            && let Some(gb) = gigabytes(&v)
        {
            limit.set_value(gb);
        }
        if let Some(v) = get(&ui, &format!("{id}:cache-max-age")).await {
            let i = AGE_CHOICES.iter().position(|c| *c == v).unwrap_or(0);
            age.set_selected(i as u32);
        }
        // Applied once the value rests: each change remounts the account.
        let pending: Rc<std::cell::Cell<Option<glib::SourceId>>> = Rc::default();
        {
            let (ui, dialog, id) = (Rc::clone(&ui), dialog.clone(), id.clone());
            limit.connect_value_notify(move |row| {
                if let Some(source) = pending.take() {
                    source.remove();
                }
                let (ui, dialog, id) = (Rc::clone(&ui), dialog.clone(), id.clone());
                let value = format!("{}G", row.value() as u64);
                let pending2 = Rc::clone(&pending);
                pending.set(Some(glib::timeout_add_local_once(
                    Duration::from_millis(800),
                    move || {
                        pending2.set(None);
                        set(&ui, &dialog, format!("{id}:cache-limit"), value);
                    },
                )));
            });
        }
        age.connect_selected_notify(move |row| {
            if let Some(value) = AGE_CHOICES.get(row.selected() as usize) {
                set(
                    &ui,
                    &dialog,
                    format!("{id}:cache-max-age"),
                    (*value).to_owned(),
                );
            }
        });
    });
    group
}

/// Lists an account's exclusions again.
type Reload = Rc<dyn Fn()>;

/// "Available Offline": the account's pinned items, their size, and
/// "Remove Offline Copy".
fn offline_group(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    status: &AccountStatus,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Available Offline"))
        .description(
            gettext(
                "Kept on this computer and never removed from the cache: they open without a connection. In use: {}.",
            )
            .replace("{}", &crate::window::human_size(status.offline_used)),
        )
        .build();
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    group.add(&list);
    let reload: Reload = {
        let cell: Rc<std::cell::RefCell<Option<Reload>>> = Rc::default();
        let (ui, dialog, list, id) = (
            Rc::clone(ui),
            dialog.clone(),
            list.clone(),
            status.account.id.clone(),
        );
        let c = Rc::clone(&cell);
        let f: Reload = Rc::new(move || {
            let (ui, dialog, list, id, cell) = (
                Rc::clone(&ui),
                dialog.clone(),
                list.clone(),
                id.clone(),
                Rc::clone(&c),
            );
            glib::spawn_future_local(async move {
                let Ok(Reply::Pins(pins)) = ui
                    .backend
                    .call(Command::PinList {
                        account: Some(id.clone()),
                    })
                    .await
                else {
                    return;
                };
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                if pins.is_empty() {
                    list.append(
                        &adw::ActionRow::builder()
                            .title(gettext("Nothing yet"))
                            .subtitle(gettext(
                                "In the file browser, right-click an item and choose “Keep Offline”.",
                            ))
                            .build(),
                    );
                }
                for pin in pins {
                    let title = if pin.path.is_empty() {
                        "/".to_owned()
                    } else {
                        pin.path.clone()
                    };
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&title))
                        .subtitle(crate::window::human_size(pin.size))
                        .build();
                    let remove = gtk::Button::builder()
                        .label(gettext("Remove Offline Copy"))
                        .valign(gtk::Align::Center)
                        .build();
                    let (ui, dialog, cell) = (Rc::clone(&ui), dialog.clone(), Rc::clone(&cell));
                    let path = nuage_core::RemotePath::new(id.clone(), &pin.path);
                    remove.connect_clicked(move |_| {
                        let (ui, dialog, cell, path) = (
                            Rc::clone(&ui),
                            dialog.clone(),
                            Rc::clone(&cell),
                            path.clone(),
                        );
                        glib::spawn_future_local(async move {
                            let commands = (
                                Command::PinRemove {
                                    path: path.clone(),
                                    force: false,
                                },
                                Command::PinRemove { path, force: true },
                            );
                            apply_rule(&ui, &dialog, commands).await;
                            if let Some(f) = cell.borrow().as_ref() {
                                f();
                            }
                        });
                    });
                    row.add_suffix(&remove);
                    list.append(&row);
                }
            });
        });
        *cell.borrow_mut() = Some(Rc::clone(&f));
        f
    };
    reload();
    group
}

fn rule_text(rule: &ExclusionRule) -> String {
    match rule {
        ExclusionRule::Path(p) => format!("/{}", p.path),
        ExclusionRule::Glob(g) => g.clone(),
    }
}

fn exclusions_group(
    ui: &Rc<Ui>,
    dialog: &adw::PreferencesDialog,
    status: &AccountStatus,
) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(gettext("Not Synced"))
        .description(gettext(
            "Items matching these stay in the cloud and are not in the folder. Folders can also be excluded from the file browser.",
        ))
        .build();
    let defaults = adw::SwitchRow::builder()
        .title(gettext("Skip Temporary Files"))
        .subtitle(glib::markup_escape_text(&DEFAULT_PATTERNS.join("  ")))
        .build();
    group.add(&defaults);
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .margin_top(12)
        .build();
    let add = adw::EntryRow::builder()
        .title(gettext("Add a Pattern (e.g. *.iso or node_modules)"))
        .show_apply_button(true)
        .build();
    let id = status.account.id.clone();
    let reload: Rc<dyn Fn()> = {
        let (ui, dialog, list, id, defaults) = (
            Rc::clone(ui),
            dialog.clone(),
            list.clone(),
            id.clone(),
            defaults.clone(),
        );
        let reload_cell: Rc<std::cell::RefCell<Option<Reload>>> = Rc::default();
        let cell = Rc::clone(&reload_cell);
        let f: Rc<dyn Fn()> = Rc::new(move || {
            let (ui, dialog, list, id, defaults, cell) = (
                Rc::clone(&ui),
                dialog.clone(),
                list.clone(),
                id.clone(),
                defaults.clone(),
                Rc::clone(&cell),
            );
            glib::spawn_future_local(async move {
                let Ok(Reply::Exclusions(rules)) = ui
                    .backend
                    .call(Command::ExcludeList {
                        account: Some(id.clone()),
                    })
                    .await
                else {
                    return;
                };
                defaults.set_active(rules.iter().any(|e| e.default));
                while let Some(child) = list.first_child() {
                    list.remove(&child);
                }
                let user: Vec<Exclusion> = rules.into_iter().filter(|e| !e.default).collect();
                list.set_visible(!user.is_empty());
                for e in user {
                    let row = adw::ActionRow::builder()
                        .title(glib::markup_escape_text(&rule_text(&e.rule)))
                        .build();
                    let remove = gtk::Button::builder()
                        .icon_name("user-trash-symbolic")
                        .tooltip_text(gettext("Sync Again"))
                        .valign(gtk::Align::Center)
                        .css_classes(["flat"])
                        .build();
                    let (ui, dialog, id, cell) =
                        (Rc::clone(&ui), dialog.clone(), id.clone(), Rc::clone(&cell));
                    remove.connect_clicked(move |_| {
                        let command = |force| Command::ExcludeRemove {
                            account: id.clone(),
                            rule: e.rule.clone(),
                            force,
                        };
                        let (ui, dialog, cell) = (Rc::clone(&ui), dialog.clone(), Rc::clone(&cell));
                        let commands = (command(false), command(true));
                        glib::spawn_future_local(async move {
                            apply_rule(&ui, &dialog, commands).await;
                            if let Some(f) = cell.borrow().as_ref() {
                                f();
                            }
                        });
                    });
                    row.add_suffix(&remove);
                    list.append(&row);
                }
            });
        });
        *reload_cell.borrow_mut() = Some(Rc::clone(&f));
        f
    };
    {
        let (ui, dialog, id, reload) = (
            Rc::clone(ui),
            dialog.clone(),
            id.clone(),
            Rc::clone(&reload),
        );
        defaults.connect_active_notify(move |row| {
            let value = if row.is_active() { "on" } else { "off" };
            // Same value as the daemon's (set while loading): nothing to do.
            let (ui, dialog, id, reload) = (
                Rc::clone(&ui),
                dialog.clone(),
                id.clone(),
                Rc::clone(&reload),
            );
            let value = value.to_owned();
            glib::spawn_future_local(async move {
                let key = format!("{}:default-exclusions", id.as_str());
                if get(&ui, &key).await.as_deref() == Some(value.as_str()) {
                    return;
                }
                if let Err(e) = ui.backend.call(Command::ConfigSet { key, value }).await {
                    dialog.add_toast(adw::Toast::new(&e));
                }
                reload();
            });
        });
    }
    {
        let (ui, dialog, id, reload) = (
            Rc::clone(ui),
            dialog.clone(),
            id.clone(),
            Rc::clone(&reload),
        );
        add.connect_apply(move |row| {
            let pattern = row.text().trim().to_owned();
            if pattern.is_empty() {
                return;
            }
            row.set_text("");
            let command = |force| Command::ExcludeAdd {
                account: id.clone(),
                rule: ExclusionRule::Glob(pattern.clone()),
                force,
            };
            let commands = (command(false), command(true));
            let (ui, dialog, reload) = (Rc::clone(&ui), dialog.clone(), Rc::clone(&reload));
            glib::spawn_future_local(async move {
                apply_rule(&ui, &dialog, commands).await;
                reload();
            });
        });
    }
    group.add(&list);
    let add_list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .margin_top(12)
        .build();
    add_list.append(&add);
    group.add(&add_list);
    reload();
    group
}

/// Sends an exclusion change; with uploads pending, asks before sending it
/// again with `force` (it remounts the account).
async fn apply_rule(ui: &Rc<Ui>, dialog: &adw::PreferencesDialog, commands: (Command, Command)) {
    let (normal, forced) = commands;
    let mut answer = ui.backend.request(normal).await;
    if let Ok(Err(Error::Busy(message))) = &answer {
        let ask = adw::AlertDialog::builder()
            .heading(gettext("Files Are Waiting to Upload"))
            .body(message)
            .build();
        ask.add_response("cancel", &gettext("_Cancel"));
        ask.add_response("apply", &gettext("_Apply Now"));
        ask.set_close_response("cancel");
        if ask.choose_future(Some(dialog)).await != "apply" {
            return;
        }
        answer = ui.backend.request(forced).await;
    }
    match answer {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => dialog.add_toast(adw::Toast::new(&e.to_string())),
        Err(e) => dialog.add_toast(adw::Toast::new(&e)),
    }
}

async fn remove_account(ui: &Rc<Ui>, dialog: &adw::PreferencesDialog, status: &AccountStatus) {
    let name = &status.account.display_name;
    let ask = adw::AlertDialog::builder()
        .heading(gettext("Remove {}?").replace("{}", name))
        .body(gettext(
            "The folder is unmounted and Nuage forgets the account. Nothing is deleted in the cloud.",
        ))
        .build();
    ask.add_response("cancel", &gettext("_Cancel"));
    ask.add_response("remove", &gettext("_Remove"));
    ask.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
    ask.set_close_response("cancel");
    if ask.choose_future(Some(dialog)).await != "remove" {
        return;
    }
    let command = |force| Command::AccountRemove {
        account: status.account.id.clone(),
        force,
    };
    let mut answer = ui.backend.request(command(false)).await;
    if let Ok(Err(Error::Busy(message))) = &answer {
        let again = adw::AlertDialog::builder()
            .heading(gettext("Remove Anyway?"))
            .body(message)
            .build();
        again.add_response("cancel", &gettext("_Cancel"));
        again.add_response("remove", &gettext("Remove _Anyway"));
        again.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        again.set_close_response("cancel");
        if again.choose_future(Some(dialog)).await != "remove" {
            return;
        }
        answer = ui.backend.request(command(true)).await;
    }
    match answer {
        Ok(Ok(_)) => {
            dialog.close();
            ui.reload().await;
            ui.toast(&gettext("{} was removed").replace("{}", name));
        }
        Ok(Err(e)) => dialog.add_toast(adw::Toast::new(&e.to_string())),
        Err(e) => dialog.add_toast(adw::Toast::new(&e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(gigabytes("20G"), Some(20.0));
        assert_eq!(gigabytes("20.0 GB"), Some(20.0));
        assert_eq!(gigabytes("1T"), Some(1024.0));
        assert_eq!(gigabytes("512M"), Some(1.0));
        assert_eq!(gigabytes("nope"), None);
    }
}
