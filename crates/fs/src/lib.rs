//! `nuage-fs`: the filesystem mounted at the user's folder.
//!
//! The user never sees rclone's mount directly. rclone mounts at a hidden
//! folder (`data/mounts/<account>`) and `nuage-fs` is mounted at the folder
//! the user chose, on top of it:
//!
//! ```text
//! ~/Cloud/Google Drive        ← nuage-fs
//!    └─ everything            → relayed to the hidden rclone mount
//! ```
//!
//! v1 relays every operation (milestone 4); exclusions (milestone 7) and
//! offline pinning (v1.1) intercept some paths before they reach rclone.
//!
//! Requests are served by several threads, so a slow read (rclone
//! downloading a file) does not block listings or other files. While the
//! hidden mount is being remounted, [`MountedFs::set_available`] makes
//! requests wait instead of showing an empty folder.

mod fusermount;
mod gate;
mod inodes;
mod relay;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use std::thread::JoinHandle;

use fuser::{Config, Session, SessionACL};

use gate::Gate;
use relay::RelayFs;

/// Threads serving FUSE requests.
const THREADS: usize = 8;

/// How long requests wait for an unavailable backing folder before failing
/// with `EIO`.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(10);

/// A mounted `nuage-fs`. Dropping it unmounts the filesystem.
pub struct MountedFs {
    /// Thread running the FUSE session (it starts the serving threads).
    session: Option<JoinHandle<io::Result<()>>>,
    mount_point: PathBuf,
    backing: PathBuf,
    gate: Arc<Gate>,
}

impl fmt::Debug for MountedFs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MountedFs")
            .field("mount_point", &self.mount_point)
            .field("backing", &self.backing)
            .finish_non_exhaustive()
    }
}

/// Options of [`mount`].
#[derive(Debug, Clone)]
pub struct MountConfig {
    /// Name shown by `mount` and file managers (`fsname`).
    pub name: String,
    /// How long requests wait for an unavailable backing folder.
    pub wait: Duration,
}

impl Default for MountConfig {
    fn default() -> Self {
        Self {
            name: "nuage".to_owned(),
            wait: DEFAULT_WAIT,
        }
    }
}

/// Mounts a relay of `backing` at `mount_point`.
///
/// Mounts through the `fusermount3` found in `PATH` (in the Flatpak, the
/// wrapper that relays to the host), like rclone.
pub fn mount(backing: &Path, mount_point: &Path, config: &MountConfig) -> io::Result<MountedFs> {
    let gate = Arc::new(Gate::new(true, config.wait));
    let fs = RelayFs::new(backing.to_owned(), Arc::clone(&gate));
    let mount_point = mount_point.canonicalize()?;
    // Commas separate options: keep them out of the name.
    let name = config.name.replace([',', '\\'], "_");
    let options = format!("fsname={name},subtype=nuage,default_permissions,nodev,nosuid,rw");
    let fd = fusermount::mount(&mount_point, &options)?;
    let mut session_config = Config::default();
    session_config.n_threads = Some(THREADS);
    // The threads share our descriptor: cloning it would open /dev/fuse,
    // which the Flatpak sandbox does not have.
    session_config.clone_fd = false;
    let session = match Session::from_fd(fs, fd, SessionACL::Owner, session_config) {
        Ok(session) => session,
        Err(e) => {
            let _ = fusermount::unmount(&mount_point);
            return Err(e);
        }
    };
    let at = mount_point.clone();
    let thread = std::thread::Builder::new()
        .name("nuage-fs".to_owned())
        .spawn(move || {
            let result = session.run();
            if let Err(e) = &result {
                tracing::error!(path = %at.display(), error = %e, "nuage-fs stopped serving");
            }
            result
        });
    let session = match thread {
        Ok(handle) => handle,
        Err(e) => {
            let _ = fusermount::unmount(&mount_point);
            return Err(e);
        }
    };
    Ok(MountedFs {
        session: Some(session),
        mount_point,
        backing: backing.to_owned(),
        gate,
    })
}

impl MountedFs {
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    pub fn backing(&self) -> &Path {
        &self.backing
    }

    /// Marks the backing folder as usable or not. While it is not, requests
    /// wait (up to [`MountConfig::wait`]) instead of failing at once.
    pub fn set_available(&self, available: bool) {
        self.gate.set(available);
    }

    /// Unmounts and waits for the serving threads to stop. Blocking.
    pub fn unmount(mut self) -> io::Result<()> {
        self.stop()
    }

    fn stop(&mut self) -> io::Result<()> {
        let Some(session) = self.session.take() else {
            return Ok(());
        };
        // Requests waiting for the backing folder must not hold the
        // unmount up.
        self.gate.set(true);
        // Once unmounted, the kernel ends the session and the threads stop.
        fusermount::unmount(&self.mount_point)?;
        session
            .join()
            .map_err(|_| io::Error::other("the nuage-fs thread panicked"))?
    }
}

impl Drop for MountedFs {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
