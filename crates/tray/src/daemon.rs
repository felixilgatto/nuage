//! Starting `nuaged` from the desktop programs (the window and the tray
//! agent), and where it writes its log.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nuage_ipc::Client;

/// Where the daemon started by the desktop programs writes its log
/// (`$XDG_DATA_HOME/nuage`, the Flatpak's own data folder in the sandbox).
pub fn log_file() -> Option<PathBuf> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/share")))?;
    let dir = data.join("nuage");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("nuaged.log"))
}

/// Whether the daemon answers on one of `sockets`.
pub async fn running(sockets: &[PathBuf], client_name: &str) -> bool {
    Client::connect_any(sockets, client_name).await.is_ok()
}

/// A program installed next to this one, or found in `PATH`.
pub fn sibling(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(name)))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from(name))
}

/// Starts `nuaged` if it does not run, and waits until it answers.
///
/// The daemon is started detached: closing the program that started it
/// does not stop the synchronisation. The log of its previous run is kept
/// next to the new one: it says why it stopped.
pub async fn ensure(sockets: &[PathBuf], client_name: &str) -> Result<(), String> {
    if running(sockets, client_name).await {
        return Ok(());
    }
    let program = sibling("nuaged");
    let log = log_file();
    if let Some(log) = &log {
        let _ = std::fs::rename(log, log.with_extension("log.old"));
    }
    let stderr = match log.as_ref().map(std::fs::File::create) {
        Some(Ok(file)) => std::process::Stdio::from(file),
        _ => std::process::Stdio::null(),
    };
    tokio::process::Command::new(&program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| format!("Cannot start {}: {e}", program.display()))?;
    for _ in 0..100 {
        if running(sockets, client_name).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(match log_file() {
        Some(log) => format!(
            "The Nuage service did not start. Its log is in {}.",
            log.display()
        ),
        None => "The Nuage service did not start.".to_owned(),
    })
}
