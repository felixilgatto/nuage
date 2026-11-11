//! The tray agent as a process: one per session, started by the window or
//! at login, stopped when the user quits Nuage.

use std::path::PathBuf;

use rustix::fs::{FlockOperation, flock};

/// Name of the agent's program.
pub const PROGRAM: &str = "nuage-tray";

/// The lock that makes the agent unique, next to the daemon's socket.
fn lock_path() -> Option<PathBuf> {
    let socket = nuage_ipc::socket_path().ok()?;
    Some(socket.parent()?.join("nuage-tray.lock"))
}

/// The running agent's claim; released when dropped (or when the process
/// ends).
#[derive(Debug)]
pub struct Instance {
    _file: std::fs::File,
}

fn open_lock() -> std::io::Result<std::fs::File> {
    let path = lock_path().ok_or_else(|| std::io::Error::other("no runtime directory"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
}

/// Claims the agent's lock. `None`: another agent runs. Never waits: an
/// agent that waited for the previous one to end would take over when the
/// user quits.
pub fn claim() -> std::io::Result<Option<Instance>> {
    let file = open_lock()?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(Instance { _file: file })),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Whether an agent runs (holds the lock).
pub fn running() -> bool {
    open_lock().is_ok_and(
        |file| match flock(&file, FlockOperation::NonBlockingLockShared) {
            Ok(()) => {
                let _ = flock(&file, FlockOperation::Unlock);
                false
            }
            Err(e) => e == rustix::io::Errno::WOULDBLOCK,
        },
    )
}

/// The socket on which the agent hears that it must end: any connection.
/// Next to the lock, so every instance of the Flatpak sees it (unlike
/// process IDs, which differ from one sandbox to the other).
pub fn stop_socket() -> Option<PathBuf> {
    Some(lock_path()?.with_file_name("nuage-tray.sock"))
}

/// Asks the running agent, if any, to end. Does not wait: see [`running`].
pub fn request_stop() {
    if let Some(path) = stop_socket() {
        // Nobody listening: no agent.
        let _ = std::os::unix::net::UnixStream::connect(path);
    }
}

/// Starts the agent, detached, unless one runs. Its output goes to
/// `nuage-tray.log` next to the daemon's log, appended.
pub fn start() -> std::io::Result<()> {
    if running() {
        return Ok(());
    }
    let program = crate::daemon::sibling(PROGRAM);
    let log = crate::daemon::log_file().map(|l| l.with_file_name("nuage-tray.log"));
    let append = |path: PathBuf| {
        // Kept small: the previous megabyte goes to `.old`.
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
            let _ = std::fs::rename(&path, path.with_extension("log.old"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    };
    let stderr = match log.map(append) {
        Some(Ok(file)) => std::process::Stdio::from(file),
        _ => std::process::Stdio::null(),
    };
    std::process::Command::new(program)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(stderr)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_agent_at_a_time() {
        let Ok(Some(first)) = super::claim() else {
            // An agent of this session already runs: nothing to check.
            return;
        };
        assert!(super::running());
        assert!(super::claim().unwrap().is_none());
        // The agent hears the stop request.
        let path = super::stop_socket().unwrap();
        let _ = std::fs::remove_file(&path);
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        super::request_stop();
        assert!(listener.accept().is_ok());
        drop(listener);
        let _ = std::fs::remove_file(&path);
        drop(first);
        assert!(!super::running());
    }
}
