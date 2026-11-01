//! The application's own small settings (view mode, start at login), in
//! `$XDG_CONFIG_HOME/nuage/gtk.ini`. The daemon's settings are not here.

use gtk::glib;

const GROUP: &str = "gtk";

fn file() -> std::path::PathBuf {
    glib::user_config_dir().join("nuage").join("gtk.ini")
}

/// A saved value, if any.
pub fn get(key: &str) -> Option<String> {
    let kf = glib::KeyFile::new();
    kf.load_from_file(file(), glib::KeyFileFlags::NONE).ok()?;
    kf.string(GROUP, key).ok().map(|s| s.to_string())
}

/// Saves a value; failures are only logged (these are conveniences).
pub fn set(key: &str, value: &str) {
    let kf = glib::KeyFile::new();
    let path = file();
    let _ = kf.load_from_file(&path, glib::KeyFileFlags::KEEP_COMMENTS);
    kf.set_string(GROUP, key, value);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = kf.save_to_file(&path) {
        tracing::debug!(error = %e, "cannot save the application settings");
    }
}
