//! The relay: every operation is replayed on the backing folder.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::{
    DirBuilderExt, DirEntryExt, FileExt, MetadataExt, OpenOptionsExt, PermissionsExt,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fuser::{
    AccessFlags, BsdFileFlags, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, InitFlags, KernelConfig, LockOwner, OpenFlags, RenameFlags, ReplyAttr,
    ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs,
    ReplyWrite, ReplyXattr, Request, TimeOrNow, WriteFlags,
};

use nuage_core::ExclusionSet;

use crate::gate::Gate;
use crate::inodes::{Inodes, ROOT};
use crate::pins::{Listed, PinView};

/// How long the kernel may cache attributes and names. Short, because the
/// backing folder changes behind our back (remote changes seen by rclone).
const TTL: Duration = Duration::from_secs(1);

/// A negative lookup answer: only the inode (0) matters.
const NO_ENTRY: FileAttr = FileAttr {
    ino: INodeNo(0),
    size: 0,
    blocks: 0,
    atime: UNIX_EPOCH,
    mtime: UNIX_EPOCH,
    ctime: UNIX_EPOCH,
    crtime: UNIX_EPOCH,
    kind: FileType::RegularFile,
    perm: 0,
    nlink: 0,
    uid: 0,
    gid: 0,
    rdev: 0,
    blksize: 4096,
    flags: 0,
};

/// Upper bound of a single read reply.
const MAX_READ: u32 = 1 << 20;

/// An open file. If rclone dies, the descriptor is dead: it is reopened by
/// path once the hidden mount is back.
#[derive(Debug)]
struct OpenFile {
    file: RwLock<Arc<File>>,
    ino: u64,
    /// `open(2)` flags, to reopen the file.
    flags: i32,
    /// Something was written through this handle.
    written: std::sync::atomic::AtomicBool,
}

impl OpenFile {
    fn new(file: File, ino: u64, flags: i32) -> Self {
        Self {
            file: RwLock::new(Arc::new(file)),
            ino,
            // Reopening must not truncate the file again.
            flags: flags & !libc::O_TRUNC,
            written: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn get(&self) -> Arc<File> {
        Arc::clone(&self.file.read().expect("open file lock"))
    }
}

/// Errors of a dead hidden mount (rclone stopped).
fn is_stale(e: &io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(libc::ENOTCONN) | Some(libc::ECONNABORTED)
    )
}

#[derive(Debug)]
enum Handle {
    File(OpenFile),
    /// A directory listing, taken at `opendir`.
    Dir(Vec<DirItem>),
}

/// The changes of a `setattr` request that are relayed.
#[derive(Debug, Clone)]
struct SetAttr {
    mode: Option<u32>,
    uid: Option<u32>,
    gid: Option<u32>,
    size: Option<u64>,
    atime: Option<TimeOrNow>,
    mtime: Option<TimeOrNow>,
}

/// What `getattr` answers from.
enum Stat {
    Meta(fs::Metadata),
    Attr(FileAttr),
}

#[derive(Debug)]
struct DirItem {
    ino: u64,
    kind: FileType,
    name: std::ffi::OsString,
}

/// Where an item lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// The hidden rclone mount.
    Backing,
    /// The local-only folder (excluded items).
    Local,
    /// The pin store (pinned items, v1.1).
    Store,
}

/// A file's state when it was last opened or closed.
///
/// Opening a file normally makes the kernel drop the pages it cached for
/// it: the file may have changed behind our back (a file command, the pin
/// sync replacing a copy). The kernel keeps them only when the file is
/// still in the state it had when they were cached.
///
/// Only for files on a local disk (the local-only folder, the pin store),
/// whose inode and change time move with every change. rclone reports the
/// modification time as change time: a remote change that keeps the size
/// and the modification time (an encrypted container, a copy that keeps
/// timestamps) would go unseen, so files on the hidden mount are always
/// read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Version {
    place: Place,
    ino: u64,
    size: u64,
    mtime: SystemTime,
    ctime: (i64, i64),
}

impl Version {
    fn of(place: Place, meta: &fs::Metadata) -> Option<Self> {
        if place == Place::Backing {
            return None;
        }
        Some(Self {
            place,
            ino: meta.ino(),
            size: meta.size(),
            mtime: meta.modified().ok()?,
            ctime: (meta.ctime(), meta.ctime_nsec()),
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(size: u64) -> Self {
        Self {
            place: Place::Store,
            ino: 1,
            size,
            mtime: UNIX_EPOCH,
            ctime: (0, 0),
        }
    }
}

/// The relay filesystem over `backing`.
#[derive(Debug)]
pub struct RelayFs {
    backing: PathBuf,
    /// Where excluded items created locally are kept (`local-only`).
    local: Option<PathBuf>,
    exclusions: Arc<RwLock<ExclusionSet>>,
    /// Where pinned items are kept (`pins`); `None`: no pinning.
    store: Option<PathBuf>,
    pins: Arc<RwLock<PinView>>,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, Arc<Handle>>>,
    next_fh: AtomicU64,
    gate: Arc<Gate>,
    /// See [`MountConfig::writes`](crate::MountConfig::writes).
    writes: Arc<AtomicU64>,
}

fn errno(e: io::Error) -> Errno {
    match e.raw_os_error() {
        Some(code) => Errno::from_i32(code),
        None => Errno::EIO,
    }
}

fn time(secs: i64, nsecs: i64) -> SystemTime {
    let nsecs = nsecs.clamp(0, 999_999_999) as u32;
    if secs >= 0 {
        UNIX_EPOCH + Duration::new(secs as u64, nsecs)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs()) + Duration::from_nanos(nsecs.into())
    }
}

fn file_type(mode: u32) -> FileType {
    match mode & libc::S_IFMT {
        libc::S_IFDIR => FileType::Directory,
        libc::S_IFLNK => FileType::Symlink,
        libc::S_IFIFO => FileType::NamedPipe,
        libc::S_IFSOCK => FileType::Socket,
        libc::S_IFCHR => FileType::CharDevice,
        libc::S_IFBLK => FileType::BlockDevice,
        _ => FileType::RegularFile,
    }
}

fn attr(ino: u64, m: &fs::Metadata) -> FileAttr {
    FileAttr {
        ino: INodeNo(ino),
        size: m.size(),
        blocks: m.blocks(),
        atime: time(m.atime(), m.atime_nsec()),
        mtime: time(m.mtime(), m.mtime_nsec()),
        ctime: time(m.ctime(), m.ctime_nsec()),
        crtime: time(m.ctime(), m.ctime_nsec()),
        kind: file_type(m.mode()),
        perm: (m.mode() & 0o7777) as u16,
        nlink: m.nlink() as u32,
        uid: m.uid(),
        gid: m.gid(),
        rdev: m.rdev() as u32,
        blksize: m.blksize() as u32,
        flags: 0,
    }
}

/// Attributes of an item known from a saved listing (offline).
fn listed_attr(ino: u64, listed: &Listed) -> FileAttr {
    // Owned by the user running the daemon, like everything it serves.
    let (uid, gid) = (
        rustix::process::getuid().as_raw(),
        rustix::process::getgid().as_raw(),
    );
    FileAttr {
        ino: INodeNo(ino),
        size: listed.size,
        blocks: listed.size.div_ceil(512),
        atime: listed.modified,
        mtime: listed.modified,
        ctime: listed.modified,
        crtime: listed.modified,
        kind: if listed.dir {
            FileType::Directory
        } else {
            FileType::RegularFile
        },
        perm: if listed.dir { 0o755 } else { 0o644 },
        nlink: if listed.dir { 2 } else { 1 },
        uid,
        gid,
        rdev: 0,
        blksize: 4096,
        flags: 0,
    }
}

/// Replaces the exclusion rules. Items kept in `local` that are no longer
/// excluded go to `backing` (rclone, and so the cloud).
pub(crate) fn apply_exclusions(
    current: &RwLock<ExclusionSet>,
    local: Option<&Path>,
    backing: &Path,
    exclusions: ExclusionSet,
) -> usize {
    *current.write().expect("exclusions lock") = exclusions;
    let Some(local) = local else {
        return 0;
    };
    let mut moved = 0;
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(entries) = fs::read_dir(local.join(&rel)) else {
            continue;
        };
        for entry in entries.flatten() {
            let child = rel.join(entry.file_name());
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if is_excluded(current, &child) {
                continue;
            }
            if is_dir {
                stack.push(child);
            } else {
                let dst = backing.join(&child);
                let result = dst
                    .parent()
                    .map_or(Ok(()), fs::create_dir_all)
                    .and_then(|()| entry.metadata())
                    .and_then(|meta| copy_file(&entry.path(), &dst, &meta))
                    .and_then(|()| fs::remove_file(entry.path()));
                match result {
                    Ok(()) => moved += 1,
                    Err(e) => {
                        tracing::warn!(path = %child.display(), error = %e, "cannot move a no longer excluded file to the cloud")
                    }
                }
            }
        }
    }
    if moved > 0 {
        tracing::info!(files = moved, "files no longer excluded moved to the cloud");
    }
    remove_empty_dirs(local);
    moved
}

fn is_excluded(set: &RwLock<ExclusionSet>, rel: &Path) -> bool {
    rel.to_str()
        .is_some_and(|r| !r.is_empty() && set.read().expect("exclusions lock").is_excluded(r))
}

/// Copies a regular file with its permissions and modification time.
fn copy_file(src: &Path, dst: &Path, meta: &fs::Metadata) -> io::Result<()> {
    fs::copy(src, dst)?;
    let file = OpenOptions::new().write(true).open(dst)?;
    file.set_modified(meta.modified()?)?;
    file.sync_all()
}

/// Removes the empty folders under `root` (not `root` itself).
fn remove_empty_dirs(root: &Path) {
    fn walk(dir: &Path, root: &Path) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) {
                    walk(&entry.path(), root);
                }
            }
        }
        if dir != root {
            let _ = fs::remove_dir(dir);
        }
    }
    walk(root, root);
}

/// Open options matching the `open(2)` flags sent by the kernel.
fn open_options(flags: i32) -> OpenOptions {
    let mut options = OpenOptions::new();
    match flags & libc::O_ACCMODE {
        libc::O_WRONLY => options.write(true),
        libc::O_RDWR => options.read(true).write(true),
        _ => options.read(true),
    };
    // The access mode is set above; creation is handled by `create`.
    let passed = flags & !(libc::O_ACCMODE | libc::O_CREAT | libc::O_EXCL | libc::O_NOCTTY);
    options.custom_flags(passed);
    options
}

impl RelayFs {
    pub fn new(
        backing: PathBuf,
        local: Option<PathBuf>,
        exclusions: Arc<RwLock<ExclusionSet>>,
        store: Option<PathBuf>,
        pins: Arc<RwLock<PinView>>,
        gate: Arc<Gate>,
        writes: Arc<AtomicU64>,
    ) -> Self {
        Self {
            backing,
            local,
            exclusions,
            store,
            pins,
            inodes: Mutex::new(Inodes::default()),
            handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
            gate,
            writes,
        }
    }

    fn inodes(&self) -> std::sync::MutexGuard<'_, Inodes> {
        self.inodes.lock().expect("inode table lock")
    }

    /// Waits for the backing folder; `EIO` if it does not come back, and
    /// `EHOSTUNREACH` at once while offline.
    fn ready(&self) -> Result<(), Errno> {
        if self.gate.is_offline() {
            return Err(Errno::EHOSTUNREACH);
        }
        if self.gate.wait() {
            Ok(())
        } else if self.gate.is_offline() {
            Err(Errno::EHOSTUNREACH)
        } else {
            Err(Errno::EIO)
        }
    }

    /// Runs `op`, which may use the backing folder (it waits for it through
    /// [`path_of`](Self::path_of)).
    ///
    /// When rclone dies, its mount answers `ENOTCONN` or `ECONNABORTED`
    /// until the daemon notices (within a second), closes the gate and
    /// remounts. Such errors are retried until the gate's timeout instead
    /// of reaching the user.
    fn with_backing<T>(&self, mut op: impl FnMut() -> Result<T, Errno>) -> Result<T, Errno> {
        let deadline = Instant::now() + self.gate.timeout();
        loop {
            match op() {
                Err(e) if (e == Errno::ENOTCONN || e == Errno::ECONNABORTED) => {
                    if Instant::now() >= deadline {
                        return Err(Errno::EIO);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                    self.ready()?;
                }
                result => return result,
            }
        }
    }

    /// Relative path of an inode.
    fn rel(&self, ino: INodeNo) -> Result<PathBuf, Errno> {
        self.inodes().path(ino.0).ok_or(Errno::ENOENT)
    }

    /// Whether `rel` is excluded from synchronisation (it then lives in the
    /// local-only folder).
    pub(crate) fn excluded(&self, rel: &Path) -> bool {
        self.local.is_some() && is_excluded(&self.exclusions, rel)
    }

    fn pins(&self) -> std::sync::RwLockReadGuard<'_, PinView> {
        self.pins.read().expect("pins lock")
    }

    /// Where `rel` lives. A pinned item is in the store once its copy is
    /// there (or, with `new`, when it is being created).
    fn place(&self, rel: &Path, new: bool) -> Place {
        if self.excluded(rel) {
            return Place::Local;
        }
        if let Some(store) = &self.store
            && self.pins().covers(rel)
            && (new || fs::symlink_metadata(store.join(rel)).is_ok())
        {
            return Place::Store;
        }
        Place::Backing
    }

    fn root(&self, place: Place) -> &Path {
        match place {
            Place::Backing => &self.backing,
            Place::Local => self.local.as_deref().expect("local-only folder"),
            Place::Store => self.store.as_deref().expect("pin store"),
        }
    }

    /// Where `rel` lives, as a full path; waits for the backing folder if
    /// that is where.
    fn path_of(&self, rel: &Path) -> Result<PathBuf, Errno> {
        self.located(rel).map(|(_, path)| path)
    }

    /// Like [`path_of`](Self::path_of), with where that is.
    fn located(&self, rel: &Path) -> Result<(Place, PathBuf), Errno> {
        let place = self.place(rel, false);
        if place == Place::Backing {
            self.ready()?;
        }
        Ok((place, self.root(place).join(rel)))
    }

    /// Counts a file written and closed, for
    /// [`MountConfig::writes`](crate::MountConfig::writes), if it is now on
    /// the hidden mount (rclone uploads it).
    fn count_upload(&self, place: Place) {
        if place == Place::Backing {
            self.writes.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Like [`count_upload`](Self::count_upload), for where `ino` is now.
    fn count_upload_of(&self, ino: u64) {
        if let Ok(rel) = self.rel(INodeNo(ino)) {
            self.count_upload(self.place(&rel, false));
        }
    }

    /// Whether the kernel may keep the pages it cached for a file being
    /// opened: only if the file is on a local disk and unchanged since it
    /// was last opened or closed (see [`Version`]). Records its state for
    /// the next open.
    fn keep_cache(&self, ino: u64, place: Place, file: &File) -> FopenFlags {
        let now = (place != Place::Backing)
            .then(|| file.metadata().ok())
            .flatten()
            .and_then(|meta| Version::of(place, &meta));
        let mut inodes = self.inodes();
        let unchanged = now.is_some() && inodes.version(ino) == now;
        inodes.set_version(ino, now);
        if unchanged {
            FopenFlags::FOPEN_KEEP_CACHE
        } else {
            FopenFlags::empty()
        }
    }

    /// Like [`path_of`](Self::path_of), for an item about to be created:
    /// makes its folder exist in the local-only folder or the store.
    fn path_for_new(&self, rel: &Path) -> Result<(Place, PathBuf), Errno> {
        let place = self.place(rel, true);
        let path = self.root(place).join(rel);
        if place == Place::Backing {
            self.ready()?;
        } else if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(errno)?;
        }
        Ok((place, path))
    }

    /// Absolute path of an inode.
    fn real(&self, ino: INodeNo) -> Result<PathBuf, Errno> {
        self.path_of(&self.rel(ino)?)
    }

    /// Relative path of `name` in directory `parent`.
    fn child(&self, parent: INodeNo, name: &OsStr) -> Result<PathBuf, Errno> {
        if name.as_encoded_bytes().contains(&b'/') || name == "." || name == ".." {
            return Err(Errno::EINVAL);
        }
        Ok(self.rel(parent)?.join(name))
    }

    /// Attributes of `rel`. Offline, items that are not pinned come from the
    /// saved listings.
    fn attr_of(&self, rel: &Path, ino: u64) -> Result<FileAttr, Errno> {
        if self.pins().is_hidden(rel) {
            return Err(Errno::ENOENT);
        }
        let place = self.place(rel, false);
        if place != Place::Backing || !self.gate.is_offline() {
            let meta = fs::symlink_metadata(self.path_of(rel)?).map_err(errno)?;
            return Ok(attr(ino, &meta));
        }
        // Offline: folders holding pinned or local items are real folders.
        for root in [self.store.as_ref(), self.local.as_ref()]
            .into_iter()
            .flatten()
        {
            if let Ok(meta) = fs::metadata(root.join(rel))
                && meta.is_dir()
            {
                return Ok(attr(ino, &meta));
            }
        }
        if rel.as_os_str().is_empty() {
            let meta = fs::metadata(&self.backing).map_err(errno)?;
            return Ok(attr(ino, &meta));
        }
        match self.pins().listed(rel) {
            Some(Some(listed)) => Ok(listed_attr(ino, listed)),
            Some(None) => Err(Errno::ENOENT),
            None => Err(Errno::EHOSTUNREACH),
        }
    }

    /// Stats `rel` and counts a kernel lookup of it.
    fn entry(&self, rel: &Path) -> Result<FileAttr, Errno> {
        let ino = self.inodes().peek(rel);
        let attr = self.attr_of(rel, ino.unwrap_or(0))?;
        let ino = self.inodes().lookup(rel);
        Ok(FileAttr {
            ino: INodeNo(ino),
            ..attr
        })
    }

    /// A pinned item went away locally: hide the remote copy rclone still
    /// shows until the engine deletes it.
    fn hide(&self, rel: &Path) {
        self.pins
            .write()
            .expect("pins lock")
            .hidden
            .insert(rel.to_owned());
    }

    fn unhide(&self, rel: &Path) {
        let mut pins = self.pins.write().expect("pins lock");
        if !pins.hidden.is_empty() {
            pins.hidden.remove(rel);
        }
    }

    /// What folder `rel` shows: the pinned items of the store, the excluded
    /// items kept locally, then rclone's listing (offline: the saved one),
    /// each name once.
    fn list(&self, rel: &Path) -> Result<Vec<DirItem>, Errno> {
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let inodes_peek =
            |child: &Path, fallback: u64| self.inodes().peek(child).unwrap_or(fallback);
        let excluded_dir = self.excluded(rel);

        // 1. The store (pinned items), unless the folder is excluded.
        // 2. The local-only folder: only the excluded items.
        let locals = [
            (Place::Store, self.store.as_ref().filter(|_| !excluded_dir)),
            (Place::Local, self.local.as_ref()),
        ];
        for (place, root) in locals {
            let Some(root) = root else { continue };
            let Ok(entries) = fs::read_dir(root.join(rel)) else {
                continue;
            };
            for entry in entries {
                let entry = entry.map_err(errno)?;
                let name = entry.file_name();
                let child = rel.join(&name);
                let wanted = match place {
                    Place::Local => self.excluded(&child),
                    _ => !self.excluded(&child),
                };
                if !wanted || !seen.insert(name.clone()) {
                    continue;
                }
                let kind = entry
                    .file_type()
                    .ok()
                    .and_then(FileType::from_std)
                    .unwrap_or(FileType::RegularFile);
                let ino = inodes_peek(&child, entry.ino());
                items.push(DirItem { ino, kind, name });
            }
        }
        if excluded_dir {
            return Ok(items);
        }

        // 3. rclone, or offline the saved listing.
        let pins = self.pins();
        let keep = |name: &OsStr| {
            let child = rel.join(name);
            !self.excluded(&child) && !pins.is_hidden(&child)
        };
        if self.gate.is_offline() {
            if let Some(listed) = pins.listings.get(rel) {
                for (i, l) in listed.iter().enumerate() {
                    if keep(&l.name) && seen.insert(l.name.clone()) {
                        let kind = if l.dir {
                            FileType::Directory
                        } else {
                            FileType::RegularFile
                        };
                        let ino = inodes_peek(&rel.join(&l.name), u64::MAX - i as u64);
                        items.push(DirItem {
                            ino,
                            kind,
                            name: l.name.clone(),
                        });
                    }
                }
            } else if items.is_empty() {
                // Nothing known about this folder.
                return Err(Errno::EHOSTUNREACH);
            }
            return Ok(items);
        }
        drop(pins);
        self.ready()?;
        let entries = match fs::read_dir(self.backing.join(rel)) {
            Ok(entries) => entries,
            // Only in the store (created in a pinned folder, not uploaded).
            Err(_) if !items.is_empty() || self.place(rel, false) == Place::Store => {
                return Ok(items);
            }
            Err(e) => return Err(errno(e)),
        };
        let pins = self.pins();
        for entry in entries {
            let entry = entry.map_err(errno)?;
            let name = entry.file_name();
            let child = rel.join(&name);
            if self.excluded(&child) || pins.is_hidden(&child) || !seen.insert(name.clone()) {
                continue;
            }
            let kind = entry
                .file_type()
                .ok()
                .and_then(FileType::from_std)
                .unwrap_or(FileType::RegularFile);
            // Known inodes keep their number; others show the backing one
            // (informational only: the kernel looks names up).
            let ino = inodes_peek(&child, entry.ino());
            items.push(DirItem { ino, kind, name });
        }
        Ok(items)
    }

    fn add_handle(&self, handle: Handle) -> FileHandle {
        let fh = self.next_fh.fetch_add(1, Ordering::Relaxed);
        self.handles
            .lock()
            .expect("handle table lock")
            .insert(fh, Arc::new(handle));
        FileHandle(fh)
    }

    fn handle(&self, fh: FileHandle) -> Result<Arc<Handle>, Errno> {
        self.handles
            .lock()
            .expect("handle table lock")
            .get(&fh.0)
            .cloned()
            .ok_or(Errno::EBADF)
    }

    /// Runs `op` on an open file. If the hidden mount died under it, waits
    /// for it to come back, reopens the file and tries again.
    fn on_file<T>(
        &self,
        fh: FileHandle,
        op: impl FnMut(&File) -> io::Result<T>,
    ) -> Result<T, Errno> {
        self.on_file_marked(fh, false, op)
    }

    /// Like [`on_file`](Self::on_file); `written` notes that `op` writes.
    fn on_file_marked<T>(
        &self,
        fh: FileHandle,
        written: bool,
        mut op: impl FnMut(&File) -> io::Result<T>,
    ) -> Result<T, Errno> {
        let handle = self.file(fh)?;
        let Handle::File(of) = &*handle else {
            unreachable!("checked by file()")
        };
        if written {
            of.written.store(true, Ordering::Relaxed);
        }
        let deadline = Instant::now() + self.gate.timeout();
        loop {
            match op(&of.get()) {
                Err(e) if is_stale(&e) => {}
                result => return result.map_err(errno),
            }
            // Reopen once the hidden mount is back, until the deadline.
            loop {
                if Instant::now() >= deadline {
                    return Err(Errno::EIO);
                }
                std::thread::sleep(Duration::from_millis(100));
                self.ready()?;
                match open_options(of.flags).open(self.real(INodeNo(of.ino))?) {
                    Ok(file) => {
                        *of.file.write().expect("open file lock") = Arc::new(file);
                        break;
                    }
                    Err(e) if is_stale(&e) => {}
                    Err(e) => return Err(errno(e)),
                }
            }
        }
    }

    /// Moves a file between the local-only folder and the backing folder
    /// (they are different filesystems): copy, then delete the source.
    fn move_across(
        &self,
        src: &Path,
        dst: &Path,
        flags: rustix::fs::RenameFlags,
    ) -> Result<(), Errno> {
        if flags.contains(rustix::fs::RenameFlags::EXCHANGE) {
            return Err(Errno::EXDEV);
        }
        let meta = fs::symlink_metadata(src).map_err(errno)?;
        // Folders and links: `mv` and file managers copy them themselves.
        if !meta.is_file() {
            return Err(Errno::EXDEV);
        }
        if flags.contains(rustix::fs::RenameFlags::NOREPLACE) && fs::symlink_metadata(dst).is_ok() {
            return Err(Errno::EEXIST);
        }
        copy_file(src, dst, &meta).map_err(errno)?;
        fs::remove_file(src).map_err(errno)
    }

    /// An open file of inode `ino`, if any.
    fn open_file_of(&self, ino: u64) -> Option<File> {
        let handles = self.handles.lock().expect("handle table lock");
        handles.values().find_map(|h| match &**h {
            Handle::File(of) if of.ino == ino => of.get().try_clone().ok(),
            _ => None,
        })
    }

    fn file(&self, fh: FileHandle) -> Result<Arc<Handle>, Errno> {
        let handle = self.handle(fh)?;
        match *handle {
            Handle::File(_) => Ok(handle),
            Handle::Dir(_) => Err(Errno::EISDIR),
        }
    }

    fn do_setattr(
        &self,
        ino: INodeNo,
        changes: SetAttr,
        fh: Option<FileHandle>,
    ) -> Result<FileAttr, Errno> {
        let SetAttr {
            mode,
            uid,
            gid,
            size,
            atime,
            mtime,
        } = changes;
        let path = self.real(ino);
        let open = fh.map(|fh| self.file(fh)).transpose()?;
        let open_file = |write: bool| -> Result<File, Errno> {
            let path = path.clone()?;
            OpenOptions::new()
                .read(!write)
                .write(write)
                .open(path)
                .map_err(errno)
        };
        let with_file = |write: bool, f: &dyn Fn(&File) -> io::Result<()>| -> Result<(), Errno> {
            match open.as_deref() {
                Some(Handle::File(of)) => f(&of.get()).map_err(errno),
                _ => f(&open_file(write)?).map_err(errno),
            }
        };

        if let Some(mode) = mode {
            fs::set_permissions(path.clone()?, fs::Permissions::from_mode(mode & 0o7777))
                .map_err(errno)?;
        }
        if uid.is_some() || gid.is_some() {
            rustix::fs::chown(
                path.clone()?,
                uid.map(rustix::fs::Uid::from_raw),
                gid.map(rustix::fs::Gid::from_raw),
            )
            .map_err(|e| Errno::from_i32(e.raw_os_error()))?;
        }
        if let Some(size) = size {
            with_file(true, &|f| f.set_len(size))?;
            match open.as_deref() {
                Some(Handle::File(of)) => of.written.store(true, Ordering::Relaxed),
                // Truncated by name: opened and closed at once.
                _ => self.count_upload_of(ino.0),
            }
        }
        if atime.is_some() || mtime.is_some() {
            let resolve = |t: TimeOrNow| match t {
                TimeOrNow::SpecificTime(t) => t,
                TimeOrNow::Now => SystemTime::now(),
            };
            let mut times = fs::FileTimes::new();
            if let Some(t) = atime {
                times = times.set_accessed(resolve(t));
            }
            if let Some(t) = mtime {
                times = times.set_modified(resolve(t));
            }
            let is_dir = fs::symlink_metadata(path.clone()?).map_err(errno)?.is_dir();
            if is_dir {
                File::open(path.clone()?)
                    .and_then(|f| f.set_times(times))
                    .map_err(errno)?;
            } else {
                with_file(false, &|f| f.set_times(times))?;
            }
        }

        let meta = match open.as_deref() {
            Some(Handle::File(of)) => of.get().metadata(),
            _ => fs::symlink_metadata(path?),
        }
        .map_err(errno)?;
        Ok(attr(ino.0, &meta))
    }
}

impl Filesystem for RelayFs {
    fn init(&mut self, _req: &Request, config: &mut KernelConfig) -> io::Result<()> {
        // The kernel drops the cached pages of an open file when a change
        // of size or modification time shows up.
        if let Err(missing) = config.add_capabilities(InitFlags::FUSE_AUTO_INVAL_DATA) {
            tracing::debug!(
                ?missing,
                "the kernel does not invalidate cached pages on changes"
            );
        }
        Ok(())
    }

    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let result = self.with_backing(|| self.entry(&self.child(parent, name)?));
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            // Applications look for many files that are not there (`.git`,
            // `.hidden`, `desktop.ini`…): the kernel remembers the answer
            // for as long as it keeps attributes (inode 0 means "no such
            // entry"). Creating the name through this mount forgets it.
            Err(Errno::ENOENT) => reply.entry(&TTL, &NO_ENTRY, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn forget(&self, _req: &Request, ino: INodeNo, nlookup: u64) {
        self.inodes().forget(ino.0, nlookup);
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let result = self.with_backing(|| {
            // An open file can be stat'ed even after it was deleted.
            if let Some(fh) = fh
                && let Ok(handle) = self.handle(fh)
                && let Handle::File(of) = &*handle
            {
                return of.get().metadata().map(Stat::Meta).map_err(errno);
            }
            match self.rel(ino) {
                Ok(rel) => return self.attr_of(&rel, ino.0).map(Stat::Attr),
                // Deleted while open: the kernel may not pass the handle.
                Err(e) => self.open_file_of(ino.0).ok_or(e)?.metadata().map_err(errno),
            }
            .map(Stat::Meta)
        });
        match result {
            Ok(Stat::Meta(meta)) => reply.attr(&TTL, &attr(ino.0, &meta)),
            Ok(Stat::Attr(attr)) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(e),
        }
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let changes = SetAttr {
            mode,
            uid,
            gid,
            size,
            atime,
            mtime,
        };
        match self.with_backing(|| self.do_setattr(ino, changes.clone(), fh)) {
            Ok(attr) => reply.attr(&TTL, &attr),
            Err(e) => reply.error(e),
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        let result = self.with_backing(|| fs::read_link(self.real(ino)?).map_err(errno));
        match result {
            Ok(target) => reply.data(target.as_os_str().as_encoded_bytes()),
            Err(e) => reply.error(e),
        }
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        let result = self.with_backing(|| {
            let rel = self.child(parent, name)?;
            fs::DirBuilder::new()
                .mode(mode & !umask & 0o7777)
                .create(self.path_for_new(&rel)?.1)
                .map_err(errno)?;
            self.unhide(&rel);
            self.entry(&rel)
        });
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let result = self.with_backing(|| {
            let rel = self.child(parent, name)?;
            let place = self.place(&rel, false);
            fs::remove_file(self.path_of(&rel)?).map_err(errno)?;
            if place == Place::Store {
                self.hide(&rel);
            }
            self.inodes().unlinked(&rel);
            Ok(())
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        let result = self.with_backing(|| {
            let rel = self.child(parent, name)?;
            let place = self.place(&rel, false);
            // Excluded or pinned items kept locally inside it count as
            // content.
            let scaffolds: Vec<PathBuf> =
                [(Place::Local, &self.local), (Place::Store, &self.store)]
                    .into_iter()
                    .filter(|(p, _)| *p != place)
                    .filter_map(|(_, root)| root.as_ref().map(|r| r.join(&rel)))
                    .collect();
            for dir in &scaffolds {
                if fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
                    return Err(Errno::ENOTEMPTY);
                }
            }
            if place == Place::Store {
                // What rclone still lists inside it must be gone too.
                let shown = self.list(&rel)?;
                if shown
                    .iter()
                    .any(|item| item.name != "." && item.name != "..")
                {
                    return Err(Errno::ENOTEMPTY);
                }
            }
            fs::remove_dir(self.path_of(&rel)?).map_err(errno)?;
            for dir in scaffolds {
                let _ = fs::remove_dir(dir);
            }
            if place == Place::Store {
                self.hide(&rel);
            }
            self.inodes().unlinked(&rel);
            Ok(())
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn symlink(
        &self,
        _req: &Request,
        parent: INodeNo,
        link_name: &OsStr,
        target: &Path,
        reply: ReplyEntry,
    ) {
        let result = self.with_backing(|| {
            let rel = self.child(parent, link_name)?;
            std::os::unix::fs::symlink(target, self.path_for_new(&rel)?.1).map_err(errno)?;
            self.unhide(&rel);
            self.entry(&rel)
        });
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        let result = self.with_backing(|| {
            let from = self.child(parent, name)?;
            let to = self.child(newparent, newname)?;
            let rflags = rustix::fs::RenameFlags::from_bits(flags.bits()).ok_or(Errno::EINVAL)?;
            let from_place = self.place(&from, false);
            let src = self.path_of(&from)?;
            // Saving over an item: it stays where it is (a pinned file
            // replaced by a temporary file stays in the store).
            let (to_place, dst) = match self.place(&to, false) {
                Place::Backing => self.path_for_new(&to)?,
                place => (place, self.root(place).join(&to)),
            };
            if from_place == to_place {
                rustix::fs::renameat_with(rustix::fs::CWD, &src, rustix::fs::CWD, &dst, rflags)
                    .map_err(|e| Errno::from_i32(e.raw_os_error()))?;
                // Excluded items kept locally inside a moved folder follow it.
                if let Some(local) = &self.local
                    && from_place == Place::Backing
                    && local.join(&from).is_dir()
                {
                    let target = local.join(&to);
                    if let Some(parent) = target.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::rename(local.join(&from), target);
                }
            } else {
                // Between places (the local-only folder, the store, rclone):
                // a temporary file saved under its final name, or the
                // reverse.
                self.move_across(&src, &dst, rflags)?;
                self.count_upload(to_place);
            }
            if from_place == Place::Store {
                self.hide(&from);
            }
            self.unhide(&to);
            let mut inodes = self.inodes();
            if rflags.contains(rustix::fs::RenameFlags::EXCHANGE) {
                inodes.exchanged(&from, &to);
            } else {
                inodes.renamed(&from, &to);
            }
            Ok(())
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn link(
        &self,
        _req: &Request,
        ino: INodeNo,
        newparent: INodeNo,
        newname: &OsStr,
        reply: ReplyEntry,
    ) {
        let result = self.with_backing(|| {
            let rel = self.child(newparent, newname)?;
            let from_place = self.place(&self.rel(ino)?, false);
            let from = self.real(ino)?;
            let (to_place, to) = self.path_for_new(&rel)?;
            if from_place != to_place {
                return Err(Errno::EXDEV);
            }
            fs::hard_link(from, to).map_err(errno)?;
            self.entry(&rel)
        });
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
            Err(e) => reply.error(e),
        }
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let result = self.with_backing(|| {
            let (place, path) = self.located(&self.rel(ino)?)?;
            let file = open_options(flags.0).open(path).map_err(errno)?;
            let keep = self.keep_cache(ino.0, place, &file);
            let fh = self.add_handle(Handle::File(OpenFile::new(file, ino.0, flags.0)));
            Ok((fh, keep))
        });
        match result {
            Ok((fh, keep)) => reply.opened(fh, keep),
            Err(e) => reply.error(e),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let result = self.on_file(fh, |file| {
            let mut buf = vec![0u8; size.min(MAX_READ) as usize];
            let mut filled = 0;
            while filled < buf.len() {
                match file.read_at(&mut buf[filled..], offset + filled as u64) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
            buf.truncate(filled);
            Ok(buf)
        });
        match result {
            Ok(data) => reply.data(&data),
            Err(e) => reply.error(e),
        }
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        let result = self.on_file_marked(fh, true, |file| file.write_all_at(data, offset));
        match result {
            Ok(()) => reply.written(data.len() as u32),
            Err(e) => reply.error(e),
        }
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        // `close(dup(fd))` makes the backing filesystem see a flush (rclone
        // reports write errors there) without closing our descriptor.
        let result = self.on_file(fh, |file| file.try_clone().map(drop));
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        // Dropping the last reference closes the backing file, which is when
        // rclone queues the upload.
        let handle = self
            .handles
            .lock()
            .expect("handle table lock")
            .remove(&fh.0);
        if let Some(Handle::File(of)) = handle.as_deref()
            && of.written.load(Ordering::Relaxed)
        {
            // The state recorded at open is gone. Not recorded here: the
            // kernel sends this after `close(2)` returned, and the file may
            // have changed since what it cached.
            self.inodes().set_version(of.ino, None);
            // Where the file is now (it may have been unpinned or included
            // since it was opened). The backing file is closed when the
            // last reference to it goes, at the latest when the reply is
            // sent.
            self.count_upload_of(of.ino);
        }
        reply.ok();
    }

    fn fsync(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        datasync: bool,
        reply: ReplyEmpty,
    ) {
        let result = self.on_file(fh, |file| {
            if datasync {
                file.sync_data()
            } else {
                file.sync_all()
            }
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    fn opendir(&self, _req: &Request, ino: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let result = self.with_backing(|| {
            let rel = self.rel(ino)?;
            let parent_ino = match rel.parent() {
                Some(p) => self.inodes().peek(p).unwrap_or(ROOT),
                None => ROOT,
            };
            let mut items = vec![
                DirItem {
                    ino: ino.0,
                    kind: FileType::Directory,
                    name: ".".into(),
                },
                DirItem {
                    ino: parent_ino,
                    kind: FileType::Directory,
                    name: "..".into(),
                },
            ];
            items.extend(self.list(&rel)?);
            Ok(self.add_handle(Handle::Dir(items)))
        });
        match result {
            Ok(fh) => reply.opened(fh, FopenFlags::empty()),
            Err(e) => reply.error(e),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let handle = match self.handle(fh) {
            Ok(h) => h,
            Err(e) => return reply.error(e),
        };
        let Handle::Dir(items) = &*handle else {
            return reply.error(Errno::ENOTDIR);
        };
        for (i, item) in items.iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(item.ino), (i + 1) as u64, item.kind, &item.name) {
                break;
            }
        }
        reply.ok();
    }

    fn releasedir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        reply: ReplyEmpty,
    ) {
        self.handles
            .lock()
            .expect("handle table lock")
            .remove(&fh.0);
        reply.ok();
    }

    fn fsyncdir(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _fh: FileHandle,
        _datasync: bool,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let result = self.with_backing(|| {
            // Offline: the disk of the store (where writes go).
            let path = match (&self.store, self.gate.is_offline()) {
                (Some(store), true) => store.clone(),
                _ => {
                    self.ready()?;
                    self.backing.clone()
                }
            };
            rustix::fs::statvfs(&path).map_err(|e| Errno::from_i32(e.raw_os_error()))
        });
        match result {
            Ok(s) => reply.statfs(
                s.f_blocks,
                s.f_bfree,
                s.f_bavail,
                s.f_files,
                s.f_ffree,
                s.f_bsize as u32,
                s.f_namemax as u32,
                s.f_frsize as u32,
            ),
            Err(e) => reply.error(e),
        }
    }

    fn access(&self, _req: &Request, ino: INodeNo, mask: AccessFlags, reply: ReplyEmpty) {
        let result = self.with_backing(|| {
            let access = rustix::fs::Access::from_bits_truncate(mask.bits() as u32);
            let rel = self.rel(ino)?;
            if self.gate.is_offline() && self.place(&rel, false) == Place::Backing {
                // Known from the saved listings: permission checks pass,
                // opening fails.
                return self.attr_of(&rel, ino.0).map(drop);
            }
            rustix::fs::access(self.path_of(&rel)?, access)
                .map_err(|e| Errno::from_i32(e.raw_os_error()))
        });
        match result {
            Ok(()) => reply.ok(),
            Err(e) => reply.error(e),
        }
    }

    // rclone has no extended attributes. `ENOSYS` makes the kernel stop
    // asking for the lifetime of the mount.
    fn getxattr(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _name: &OsStr,
        _size: u32,
        reply: ReplyXattr,
    ) {
        reply.error(Errno::ENOSYS);
    }

    fn listxattr(&self, _req: &Request, _ino: INodeNo, _size: u32, reply: ReplyXattr) {
        reply.error(Errno::ENOSYS);
    }

    fn setxattr(
        &self,
        _req: &Request,
        _ino: INodeNo,
        _name: &OsStr,
        _value: &[u8],
        _flags: i32,
        _position: u32,
        reply: ReplyEmpty,
    ) {
        reply.error(Errno::ENOSYS);
    }

    fn removexattr(&self, _req: &Request, _ino: INodeNo, _name: &OsStr, reply: ReplyEmpty) {
        reply.error(Errno::ENOSYS);
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        let result = self.with_backing(|| {
            let rel = self.child(parent, name)?;
            let mut options = open_options(flags);
            options.create(true).mode(mode & !umask & 0o7777);
            if flags & libc::O_EXCL != 0 {
                options.create_new(true);
            }
            // POSIX allows creating a file opened read-only, but `OpenOptions`
            // refuses to create without write access. The kernel only sends
            // reads for a read-only handle, so a writable descriptor is safe.
            options.write(true);
            let path = self.path_for_new(&rel)?.1;
            let file = options.open(path).map_err(errno)?;
            self.unhide(&rel);
            let meta = file.metadata().map_err(errno)?;
            let ino = self.inodes().lookup(&rel);
            // Reopened without O_CREAT/O_EXCL, but writable like here.
            let reopen = (flags & !libc::O_ACCMODE) | libc::O_RDWR;
            let of = OpenFile::new(file, ino, reopen);
            // Even empty, a new file is uploaded.
            of.written.store(true, Ordering::Relaxed);
            let fh = self.add_handle(Handle::File(of));
            Ok((attr(ino, &meta), fh))
        });
        match result {
            Ok((attr, fh)) => reply.created(&TTL, &attr, Generation(0), fh, FopenFlags::empty()),
            Err(e) => reply.error(e),
        }
    }
}
