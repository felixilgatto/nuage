//! GTK4 + libadwaita user interface: onboarding, main window, browser,
//! preferences.
//!
//! **Skeleton**: only an empty main window for now. Every user-visible
//! string goes through gettext.

use gettextrs::gettext;

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

/// Builds the (still empty) main window.
pub fn build_main_window(app: &adw::Application) -> adw::ApplicationWindow {
    let page = adw::StatusPage::builder()
        .icon_name("folder-remote-symbolic")
        .title(gettext("No Accounts"))
        .description(gettext("Nuage is under construction."))
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&page));

    adw::ApplicationWindow::builder()
        .application(app)
        .title("Nuage")
        .default_width(640)
        .default_height(480)
        .content(&toolbar)
        .build()
}
