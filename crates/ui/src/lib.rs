//! GTK 4 + libadwaita user interface: setup, main window, and (later) the
//! file browser and preferences.
//!
//! The interface is a client of `nuaged` ([`Backend`]); it holds no state
//! of its own beyond what it shows. Every user-visible string goes through
//! gettext.

mod backend;
pub mod onboarding;
mod window;

use gettextrs::gettext;

pub use backend::{Backend, BackendEvent, glib_sleep, log_file};
pub use window::{Ui, human_size, open_folder, state_icon, state_text};

/// gettext domain of the application.
pub const GETTEXT_PACKAGE: &str = "nuage";

/// Initialises translations from `locale_dir` (e.g. `/app/share/locale`).
/// Failures are not fatal: the interface falls back to English.
///
/// `setlocale` is not called here: GTK does it when it initialises.
pub fn init_i18n(locale_dir: &str) {
    let _ = gettextrs::bindtextdomain(GETTEXT_PACKAGE, locale_dir);
    let _ = gettextrs::bind_textdomain_codeset(GETTEXT_PACKAGE, "UTF-8");
    let _ = gettextrs::textdomain(GETTEXT_PACKAGE);
}

/// The "About" dialog.
pub fn show_about(parent: &impl gtk::prelude::IsA<gtk::Widget>) {
    let about = adw::AboutDialog::builder()
        .application_name("Nuage")
        .application_icon(nuage_core::APP_ID)
        .version(nuage_core::VERSION)
        .comments(gettext("Your cloud storage as a regular folder."))
        .website("https://github.com/felixilgatto/nuage")
        .issue_url("https://github.com/felixilgatto/nuage/issues")
        .license_type(gtk::License::MitX11)
        .build();
    adw::prelude::AdwDialogExt::present(&about, Some(parent));
}
