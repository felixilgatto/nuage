//! `nuage-fuse-relay`: mounts a FUSE filesystem on behalf of the Flatpak.
//!
//! Inside the sandbox, `/app/bin/fusermount3` is a wrapper. When rclone or
//! `nuage-fs` mount, libfuse runs it with a socket in `$_FUSE_COMMFD` and
//! expects the `/dev/fuse` descriptor back on that socket. The wrapper runs
//! this program on the host (`flatpak-spawn --host --forward-fd`).
//!
//! The host's `fusermount3` could use the forwarded socket directly, but on
//! Ubuntu 26.04 its AppArmor profile refuses sockets inherited from a
//! sandboxed process. So this relay creates its own socket pair, runs the
//! host's `fusermount3` with it, receives the descriptor and passes it back
//! through the sandbox's socket.
//!
//! Without `$_FUSE_COMMFD` (unmount, `--version`…), the arguments go
//! straight to the host's `fusermount3`.
//!
//! The host `fusermount3` is `$NUAGE_HOST_FUSERMOUNT`, else the first
//! `fusermount3` in `PATH` that is not this program.

use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, SocketFlags, SocketType,
};

const COMM_ENV: &str = "_FUSE_COMMFD";
/// Set for the `fusermount3` this program runs, to detect loops (a wrapper
/// found again in `PATH`).
const LOOP_ENV: &str = "NUAGE_FUSE_RELAY_ACTIVE";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("nuage-fuse-relay: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    if std::env::var_os(LOOP_ENV).is_some() {
        return Err(
            "called by itself: the fusermount3 it found is a wrapper; set NUAGE_HOST_FUSERMOUNT"
                .to_owned(),
        );
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let fusermount = host_fusermount()?;
    let Some(comm) = std::env::var(COMM_ENV).ok().filter(|v| !v.is_empty()) else {
        // Nothing to relay.
        let status = Command::new(&fusermount)
            .args(&args)
            .env(LOOP_ENV, "1")
            .status()
            .map_err(|e| format!("cannot run {}: {e}", fusermount.display()))?;
        return Ok(exit_code(status));
    };
    let comm = adopt_socket(&comm)?;

    // Our own socket pair: `theirs` is inherited by fusermount3 (no
    // close-on-exec), `ours` is kept here.
    let (ours, theirs) = rustix::net::socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::empty(),
        None,
    )
    .map_err(|e| format!("socketpair: {e}"))?;
    rustix::io::fcntl_setfd(&ours, rustix::io::FdFlags::CLOEXEC)
        .map_err(|e| format!("fcntl: {e}"))?;
    rustix::io::fcntl_setfd(&comm, rustix::io::FdFlags::CLOEXEC)
        .map_err(|e| format!("fcntl: {e}"))?;

    let mut child = Command::new(&fusermount)
        .args(&args)
        .env(COMM_ENV, theirs.as_raw_fd().to_string())
        .env(LOOP_ENV, "1")
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", fusermount.display()))?;
    drop(theirs);

    // fusermount3 sends one byte with the descriptor, or closes the socket
    // if the mount failed (it then prints why and exits non-zero).
    match receive_fd(&ours) {
        Ok(Some((byte, fd))) => send_fd(&comm, byte, &fd)?,
        Ok(None) => {}
        Err(e) => {
            let _ = child.kill();
            return Err(e);
        }
    }
    let status = child
        .wait()
        .map_err(|e| format!("waiting for fusermount3: {e}"))?;
    Ok(exit_code(status))
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
        None => ExitCode::FAILURE,
    }
}

/// Takes ownership of the socket whose number is in `$_FUSE_COMMFD`.
fn adopt_socket(value: &str) -> Result<OwnedFd, String> {
    let fd: i32 = value
        .parse()
        .ok()
        .filter(|&n| n > 2)
        .ok_or_else(|| format!("invalid {COMM_ENV}: {value:?}"))?;
    // The descriptor must be open before we claim it.
    if !Path::new(&format!("/proc/self/fd/{fd}")).exists() {
        return Err(format!("{COMM_ENV}={fd} is not an open descriptor"));
    }
    // SAFETY: libfuse (through flatpak-spawn --forward-fd) gives this process
    // descriptor `fd` for exactly this purpose; it was checked to be open,
    // nothing else in this program refers to it, and it is adopted once.
    #[allow(unsafe_code)]
    let owned = unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
    let stat = rustix::fs::fstat(&owned).map_err(|e| format!("fstat {COMM_ENV}: {e}"))?;
    if rustix::fs::FileType::from_raw_mode(stat.st_mode) != rustix::fs::FileType::Socket {
        return Err(format!("{COMM_ENV}={fd} is not a socket"));
    }
    Ok(owned)
}

/// Receives one byte and one descriptor; `None` if the peer closed the
/// socket without sending anything.
fn receive_fd(socket: &OwnedFd) -> Result<Option<(u8, OwnedFd)>, String> {
    let mut byte = [0u8; 1];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    let received = loop {
        let mut iov = [IoSliceMut::new(&mut byte)];
        match rustix::net::recvmsg(
            socket.as_fd(),
            &mut iov,
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        ) {
            Ok(r) => break r,
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(format!("receiving from fusermount3: {e}")),
        }
    };
    if received.bytes == 0 {
        return Ok(None);
    }
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(mut fds) = message
            && let Some(fd) = fds.next()
        {
            return Ok(Some((byte[0], fd)));
        }
    }
    Err("fusermount3 sent no descriptor".to_owned())
}

fn send_fd(socket: &OwnedFd, byte: u8, fd: &OwnedFd) -> Result<(), String> {
    let fds = [fd.as_fd()];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
        return Err("control buffer too small".to_owned());
    }
    let data = [byte];
    let iov = [IoSlice::new(&data)];
    loop {
        match rustix::net::sendmsg(socket.as_fd(), &iov, &mut control, SendFlags::empty()) {
            Ok(_) => return Ok(()),
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(format!("passing the descriptor back: {e}")),
        }
    }
}

/// The host's real `fusermount3`.
fn host_fusermount() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("NUAGE_HOST_FUSERMOUNT").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    // When tests install this program as `fusermount3`, skip it.
    let me = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok());
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .chain([PathBuf::from("/usr/bin"), PathBuf::from("/bin")])
        .map(|dir| dir.join("fusermount3"))
        .find(|p| p.is_file() && p.canonicalize().ok() != me)
        .ok_or_else(|| "fusermount3 was not found on the host; install fuse3".to_owned())
}
