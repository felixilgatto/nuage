//! `nuage-fs`: the filesystem mounted at the user's folder.
//!
//! The user never sees rclone's mount directly. rclone mounts at a hidden
//! folder (`data/mounts/<account>`) and `nuage-fs` is mounted at the folder
//! the user chose, on top of it:
//!
//! ```text
//! ~/Cloud/Google Drive        ← nuage-fs
//!    ├─ excluded, created here → the local-only folder
//!    ├─ pinned                 → the pin store (v1.1)
//!    └─ everything else        → relayed to the hidden rclone mount
//! ```
//!
//! v1 relays every operation (milestone 4); exclusions (milestone 7) and
//! offline pinning (v1.1, [`PinView`]) intercept some paths before they
//! reach rclone. Offline ([`MountedFs::set_offline`]), pinned items keep
//! working and the others are listed from the engine's saved listings.
//!
//! Requests are served by several threads, so a slow read (rclone
//! downloading a file) does not block listings or other files. While the
//! hidden mount is being remounted, [`MountedFs::set_available`] makes
//! requests wait instead of showing an empty folder.

mod fusermount;
mod gate;
mod inodes;
mod pins;
mod relay;

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use std::thread::JoinHandle;

use fuser::{Config, Session, SessionACL};

pub use nuage_core::ExclusionSet;
pub use pins::{Listed, PinView};

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
    exclusions: Arc<RwLock<ExclusionSet>>,
    local_only: Option<PathBuf>,
    pins: Arc<RwLock<PinView>>,
    store: Option<PathBuf>,
    /// fuse connection number, to abort the session if it does not end.
    connection: Option<u32>,
}

/// How long the threads may take to stop once unmounted.
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Folder where items matching `exclusions` live instead of the backing
    /// folder (`data/local-only/<account>`). `None`: no exclusion.
    pub local_only: Option<PathBuf>,
    pub exclusions: ExclusionSet,
    /// Folder holding the copies of pinned items (`data/pins/<account>`).
    /// `None`: no pinning.
    pub store: Option<PathBuf>,
    pub pins: PinView,
    /// Mounted while the provider cannot be reached (offline start).
    pub offline: bool,
}

impl Default for MountConfig {
    fn default() -> Self {
        Self {
            name: "nuage".to_owned(),
            wait: DEFAULT_WAIT,
            local_only: None,
            exclusions: ExclusionSet::default(),
            store: None,
            pins: PinView::default(),
            offline: false,
        }
    }
}

/// Mounts a relay of `backing` at `mount_point`.
///
/// Mounts through the `fusermount3` found in `PATH` (in the Flatpak, the
/// wrapper that relays to the host), like rclone.
pub fn mount(backing: &Path, mount_point: &Path, config: &MountConfig) -> io::Result<MountedFs> {
    let gate = Arc::new(Gate::new(!config.offline, config.wait));
    gate.set_offline(config.offline);
    for dir in [&config.local_only, &config.store].into_iter().flatten() {
        std::fs::create_dir_all(dir)?;
    }
    let exclusions = Arc::new(RwLock::new(config.exclusions.clone()));
    let pins = Arc::new(RwLock::new(config.pins.clone()));
    let fs = RelayFs::new(
        backing.to_owned(),
        config.local_only.clone(),
        Arc::clone(&exclusions),
        config.store.clone(),
        Arc::clone(&pins),
        Arc::clone(&gate),
    );
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
    // The root is served now: its device names the fuse connection.
    let connection = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&mount_point)
            .ok()
            .map(|m| fusermount::connection_of(m.dev()))
    };
    Ok(MountedFs {
        session: Some(session),
        connection,
        mount_point,
        backing: backing.to_owned(),
        gate,
        exclusions,
        local_only: config.local_only.clone(),
        pins,
        store: config.store.clone(),
    })
}

impl MountedFs {
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }

    pub fn backing(&self) -> &Path {
        &self.backing
    }

    /// Offline (the provider cannot be reached): pinned items work from the
    /// store, others are listed from the saved listings and cannot be
    /// opened (`EHOSTUNREACH`), without waiting.
    pub fn set_offline(&self, offline: bool) {
        self.gate.set_offline(offline);
    }

    pub fn is_offline(&self) -> bool {
        self.gate.is_offline()
    }

    /// Replaces the pins. Items deleted here since the engine computed
    /// `view` stay hidden while their copy is gone.
    pub fn set_pins(&self, mut view: PinView) {
        let mut current = self.pins.write().expect("pins lock");
        if let Some(store) = &self.store {
            for rel in &current.hidden {
                if std::fs::symlink_metadata(store.join(rel)).is_err() && view.covers(rel) {
                    view.hidden.insert(rel.clone());
                }
            }
        }
        *current = view;
    }

    /// Marks the backing folder as usable or not. While it is not, requests
    /// wait (up to [`MountConfig::wait`]) instead of failing at once.
    pub fn set_available(&self, available: bool) {
        self.gate.set(available);
    }

    /// Replaces the exclusion rules. Items kept locally that are no longer
    /// excluded are moved to the backing folder. Blocking.
    pub fn set_exclusions(&self, exclusions: ExclusionSet) {
        relay::apply_exclusions(
            &self.exclusions,
            self.local_only.as_deref(),
            &self.backing,
            exclusions,
        );
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
        // A busy folder (open in Files, a shell inside it…) is detached
        // instead, and the kernel would keep the session for as long as it
        // is used: end it.
        if fusermount::unmount(&self.mount_point)? == fusermount::Unmounted::Lazily
            && let Some(connection) = self.connection
            && let Err(e) = fusermount::abort(connection)
        {
            tracing::warn!(error = %e, "cannot end the FUSE session of a busy folder");
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        while !session.is_finished() {
            if Instant::now() > deadline {
                // Never hang: the session ends with the process.
                tracing::warn!(
                    path = %self.mount_point.display(),
                    "the folder is still in use; nuage-fs stops serving it when the daemon exits"
                );
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
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
