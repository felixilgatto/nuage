//! Mounting through `fusermount3`, like rclone does.
//!
//! `fuser` would first open `/dev/fuse` itself, which the Flatpak sandbox
//! does not allow. Running `fusermount3` (in the Flatpak, the wrapper that
//! relays to the host) and receiving the `/dev/fuse` descriptor on a socket
//! works everywhere, and both layers of a mount go the same way.

use std::ffi::OsString;
use std::io::{self, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::path::Path;
use std::process::{Command, Stdio};

use rustix::net::{
    AddressFamily, RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SocketFlags, SocketType,
};

/// `fusermount3`, or `$FUSERMOUNT_PATH` (the variable `fuser` honours).
fn binary() -> OsString {
    std::env::var_os("FUSERMOUNT_PATH")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "fusermount3".into())
}

/// Mounts at `mount_point` with `options` (`-o` syntax) and returns the
/// `/dev/fuse` descriptor of the new mount.
pub fn mount(mount_point: &Path, options: &str) -> io::Result<OwnedFd> {
    // `theirs` is inherited by fusermount3 (no close-on-exec).
    let (ours, theirs) = rustix::net::socketpair(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::empty(),
        None,
    )?;
    rustix::io::fcntl_setfd(&ours, rustix::io::FdFlags::CLOEXEC)?;
    let bin = binary();
    let child = Command::new(&bin)
        .arg("-o")
        .arg(options)
        .arg("--")
        .arg(mount_point)
        .env("_FUSE_COMMFD", theirs.as_raw_fd().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("cannot run {}: {e}", bin.to_string_lossy()),
            )
        })?;
    drop(theirs);

    let received = receive_fd(&ours);
    let output = child.wait_with_output()?;
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match received? {
        Some(fd) if output.status.success() => Ok(fd),
        _ => Err(io::Error::other(if stderr.is_empty() {
            format!("fusermount3 failed ({})", output.status)
        } else {
            stderr
        })),
    }
}

/// How a mount went away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unmounted {
    Now,
    /// Busy: detached with `-z`. The kernel keeps the session until the
    /// last process using the folder lets go of it.
    Lazily,
}

/// Unmounts with `fusermount3 -u`, then `-u -z` (lazy) if it is busy.
pub fn unmount(mount_point: &Path) -> io::Result<Unmounted> {
    let bin = binary();
    let mut last = String::new();
    for lazy in [false, true] {
        let mut cmd = Command::new(&bin);
        cmd.arg("-u").arg("-q");
        if lazy {
            cmd.arg("-z");
        }
        let output = cmd
            .arg("--")
            .arg(mount_point)
            .stdin(Stdio::null())
            .output()?;
        if output.status.success() {
            return Ok(if lazy {
                Unmounted::Lazily
            } else {
                Unmounted::Now
            });
        }
        last = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    }
    Err(io::Error::other(format!(
        "cannot unmount {}: {last}",
        mount_point.display()
    )))
}

/// One descriptor sent by fusermount3; `None` if it closed the socket.
fn receive_fd(socket: &OwnedFd) -> io::Result<Option<OwnedFd>> {
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
            Err(e) => return Err(e.into()),
        }
    };
    if received.bytes == 0 {
        return Ok(None);
    }
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(mut fds) = message
            && let Some(fd) = fds.next()
        {
            return Ok(Some(fd));
        }
    }
    Ok(None)
}

/// Ends a FUSE session at once, through the fuse control filesystem
/// (`/sys/fs/fuse/connections/<id>/abort`, writable by the mount's owner).
/// Processes still using a lazily unmounted folder then get errors instead
/// of keeping the session alive. Best effort: it may be missing, e.g. in a
/// sandbox.
pub fn abort(connection: u32) -> io::Result<()> {
    std::fs::write(format!("/sys/fs/fuse/connections/{connection}/abort"), "1")
}

/// The fuse connection number of a mount: the minor of its device.
pub fn connection_of(dev: u64) -> u32 {
    ((dev & 0xff) | ((dev >> 12) & 0xfff00)) as u32
}
