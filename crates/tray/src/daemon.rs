//! Starting `nuaged` from the desktop programs (the window and the tray
//! agent), and where it writes its log.

use std::path::{Path, PathBuf};
use std::time::Duration;

use nuage_core::Event;
use nuage_ipc::{Client, IpcError};

/// What a desktop program hears from the daemon.
#[derive(Debug, Clone)]
pub enum Link {
    /// Connected (again): what was known of the daemon is out of date.
    Connected,
    /// The daemon cannot be reached, and why.
    Disconnected(String),
    Event(Event),
}

/// Follows the daemon's events, forever, reconnecting as needed: one
/// [`Link::Connected`] per connection, one [`Link::Disconnected`] when it
/// cannot be reached (again). Ends when `tx` is closed.
pub async fn follow(sockets: Vec<PathBuf>, client_name: String, tx: async_channel::Sender<Link>) {
    let mut connected = None;
    loop {
        match Client::connect_any(&sockets, &client_name).await {
            Ok(client) => match client.subscribe().await {
                Ok(mut events) => {
                    if tx.send(Link::Connected).await.is_err() {
                        return;
                    }
                    connected = Some(true);
                    loop {
                        match events.next_event().await {
                            Ok(Some(event)) => {
                                if tx.send(Link::Event(event)).await.is_err() {
                                    return;
                                }
                            }
                            Ok(None) | Err(IpcError::ConnectionClosed) => break,
                            Err(e) => {
                                tracing::debug!(error = %e, "event stream failed");
                                break;
                            }
                        }
                    }
                }
                Err(e) => tracing::debug!(error = %e, "cannot subscribe"),
            },
            Err(e) => {
                if connected != Some(false) {
                    connected = Some(false);
                    if tx.send(Link::Disconnected(e.to_string())).await.is_err() {
                        return;
                    }
                }
            }
        }
        if connected == Some(true) {
            connected = None;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

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

/// What to do when the daemon left running by another version of Nuage
/// cannot be talked to.
const OTHER_VERSION: &str = "Nuage was updated, but its previous version is still running. \
     Quit Nuage from its icon in the system tray, or log out and back in, then open Nuage again.";

/// Starts `nuaged` if it does not run, and waits until it answers.
///
/// The daemon is started detached: closing the program that started it
/// does not stop the synchronisation. The log of its previous run is kept
/// next to the new one: it says why it stopped.
pub async fn ensure(sockets: &[PathBuf], client_name: &str) -> Result<(), String> {
    match Client::connect_any(sockets, client_name).await {
        Ok(_) => return Ok(()),
        // The daemon of another version of Nuage answers (after an
        // update): a second one could not take its place, and would
        // replace its log.
        Err(IpcError::ProtocolMismatch(e)) => {
            tracing::warn!(error = %e, "the running Nuage service is from another version");
            return Err(OTHER_VERSION.to_owned());
        }
        Err(_) => {}
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

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    /// The daemon of another version answers: no second one is started.
    #[test]
    fn other_version_is_not_replaced() {
        let dir = std::env::temp_dir().join(format!("nuage-tray-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nuaged.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut hello = String::new();
            BufReader::new(&stream).read_line(&mut hello).unwrap();
            (&stream)
                .write_all(b"{\"type\":\"rejected\",\"protocol_version\":999,\"message\":\"no\"}\n")
                .unwrap();
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.block_on(super::ensure(std::slice::from_ref(&path), "test"));
        server.join().unwrap();
        assert_eq!(result, Err(super::OTHER_VERSION.to_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
