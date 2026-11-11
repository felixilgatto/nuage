//! The tray agent as a process: one per session, started by the window or
//! at login, stopped when the user quits Nuage.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use rustix::fs::{FlockOperation, flock};

/// Name of the agent's program.
pub const PROGRAM: &str = "nuage-tray";

/// The lock that makes the agent unique, next to the daemon's socket. It
/// holds the agent's PID.
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
/// user quits (the window's Restart stops the old agent before starting the
/// new version).
pub fn claim() -> std::io::Result<Option<Instance>> {
    let mut file = open_lock()?;
    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => {}
        Err(rustix::io::Errno::WOULDBLOCK) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    file.set_len(0)?;
    write!(file, "{}", std::process::id())?;
    file.flush()?;
    Ok(Some(Instance { _file: file }))
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

/// Stops the running agent, if any, and waits (up to 2 s) until it is gone.
pub fn stop() {
    let Some(pid) = lock_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| t.trim().parse::<i32>().ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    let proc = PathBuf::from(format!("/proc/{}", pid.as_raw_nonzero()));
    // Only if it still is the agent: the PID may have been reused.
    let cmdline = std::fs::read(proc.join("cmdline")).unwrap_or_default();
    let is_agent = cmdline
        .split(|&b| b == 0)
        .next()
        .is_some_and(|a| a.ends_with(PROGRAM.as_bytes()));
    if !is_agent || u32::try_from(pid.as_raw_nonzero().get()).ok() == Some(std::process::id()) {
        return;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    for _ in 0..20 {
        if !alive(&proc) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Whether the process of `/proc/<pid>` still runs. An ended child of this
/// process stays as a zombie until reaped: it counts as gone.
fn alive(proc: &std::path::Path) -> bool {
    std::fs::read_to_string(proc.join("stat")).is_ok_and(|stat| {
        // `pid (comm) state …`: the state follows the last parenthesis.
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z" && state != "X")
    })
}

#[cfg(test)]
mod tests {
    use super::alive;

    #[test]
    fn one_agent_at_a_time() {
        let Ok(Some(first)) = super::claim() else {
            // An agent of this session already runs: nothing to check.
            return;
        };
        assert!(super::running());
        assert!(super::claim().unwrap().is_none());
        drop(first);
        assert!(!super::running());
    }

    #[test]
    fn this_process_is_alive() {
        assert!(alive(std::path::Path::new("/proc/self")));
        assert!(!alive(std::path::Path::new("/proc/no-such-process")));
    }
}
