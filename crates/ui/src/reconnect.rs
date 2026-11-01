//! Signing in to an account again (expired or revoked access, new OAuth
//! client), from a notification, the main window or the preferences.

use std::rc::Rc;

use adw::prelude::*;
use gettextrs::gettext;
use gtk::{gio, glib};

use nuage_core::{AccountId, Command, Reply};

use crate::window::Ui;

/// Starts the sign-in in the browser and waits for it in a dialog over
/// `parent`. Returns whether the account is signed in again.
pub async fn reconnect(ui: &Rc<Ui>, account: &AccountId, parent: &impl IsA<gtk::Widget>) -> bool {
    let name = ui
        .accounts()
        .into_iter()
        .find(|a| &a.account.id == account)
        .map_or_else(|| account.as_str().to_owned(), |a| a.account.display_name);
    let sign_in = match ui
        .backend
        .call(Command::AccountReconnect {
            account: account.clone(),
            token: None,
        })
        .await
    {
        Ok(Reply::SignIn(s)) => s,
        // Signed in with what the daemon had (nothing to do in a browser).
        Ok(_) => return true,
        Err(message) => {
            ui.toast(&message);
            return false;
        }
    };

    let launcher = gtk::UriLauncher::new(&sign_in.url);
    let window = parent.root().and_downcast::<gtk::Window>();
    launcher.launch(window.as_ref(), gio::Cancellable::NONE, |result| {
        if let Err(e) = result {
            tracing::warn!(error = %e, "cannot open the browser");
        }
    });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.append(
        &gtk::Spinner::builder()
            .spinning(true)
            .width_request(32)
            .height_request(32)
            .build(),
    );
    let copy = gtk::Button::builder()
        .label(gettext("Copy Sign-In Link"))
        .halign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    {
        let url = sign_in.url.clone();
        copy.connect_clicked(move |b| b.clipboard().set_text(&url));
    }
    content.append(&copy);
    let dialog = adw::AlertDialog::builder()
        .heading(gettext("Sign In to {}").replace("{}", &name))
        .body(gettext("Waiting for sign-in in your browser…"))
        .extra_child(&content)
        .build();
    dialog.add_response("cancel", &gettext("_Cancel"));
    dialog.set_close_response("cancel");

    let (done_tx, done_rx) = async_channel::bounded::<Result<(), String>>(1);
    {
        let ui = Rc::clone(ui);
        let session = sign_in.session.clone();
        glib::spawn_future_local(async move {
            let result = ui
                .backend
                .call(Command::SignInWait { session })
                .await
                .map(drop);
            let _ = done_tx.send(result).await;
        });
    }
    let cancelled = {
        let ui = Rc::clone(ui);
        let session = sign_in.session.clone();
        let dialog = dialog.clone();
        let parent = parent.clone().upcast::<gtk::Widget>();
        async move {
            if dialog.choose_future(Some(&parent)).await == "cancel" {
                let _ = ui.backend.call(Command::SignInCancel { session }).await;
            }
        }
    };
    let cancelled = std::pin::pin!(cancelled);
    let finished = std::pin::pin!(done_rx.recv());
    match futures_select(cancelled, finished).await {
        Some(Ok(Ok(()))) => {
            dialog.force_close();
            ui.come_back(
                &gettext("{} is connected again").replace("{}", &name),
                &gettext("Files sync again."),
            );
            ui.toast(&gettext("{} is connected again").replace("{}", &name));
            ui.reload().await;
            true
        }
        Some(Ok(Err(message))) => {
            dialog.force_close();
            ui.toast(&message);
            false
        }
        // Cancelled, or the wait was dropped.
        _ => false,
    }
}

/// Waits for the first of two futures; `None` when it is `a`.
async fn futures_select<A, B>(a: A, b: B) -> Option<B::Output>
where
    A: std::future::Future<Output = ()> + Unpin,
    B: std::future::Future + Unpin,
{
    let mut a = a;
    let mut b = b;
    std::future::poll_fn(move |cx| {
        if let std::task::Poll::Ready(v) = std::pin::Pin::new(&mut b).poll(cx) {
            return std::task::Poll::Ready(Some(v));
        }
        if std::pin::Pin::new(&mut a).poll(cx).is_ready() {
            return std::task::Poll::Ready(None);
        }
        std::task::Poll::Pending
    })
    .await
}
