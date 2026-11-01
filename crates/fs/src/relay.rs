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
    Generation, INodeNo, LockOwner, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate, ReplyData,
    ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, ReplyXattr,
    Request, TimeOrNow, WriteFlags,
};

use nuage_core::ExclusionSet;

use crate::gate::Gate;
use crate::inodes::{Inodes, ROOT};

/// How long the kernel may cache attributes and names. Short, because the
/// backing folder changes behind our back (remote changes seen by rclone).
const TTL: Duration = Duration::from_secs(1);

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
}

impl OpenFile {
    fn new(file: File, ino: u64, flags: i32) -> Self {
        Self {
            file: RwLock::new(Arc::new(file)),
            ino,
            // Reopening must not truncate the file again.
            flags: flags & !libc::O_TRUNC,
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

#[derive(Debug)]
struct DirItem {
    ino: u64,
    kind: FileType,
    name: std::ffi::OsString,
}

/// The relay filesystem over `backing`.
#[derive(Debug)]
pub struct RelayFs {
    backing: PathBuf,
    /// Where excluded items created locally are kept (`local-only`).
    local: Option<PathBuf>,
    exclusions: Arc<RwLock<ExclusionSet>>,
    inodes: Mutex<Inodes>,
    handles: Mutex<HashMap<u64, Arc<Handle>>>,
    next_fh: AtomicU64,
    gate: Arc<Gate>,
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

/// Replaces the exclusion rules. Items kept in `local` that are no longer
/// excluded go to `backing` (rclone, and so the cloud).
pub(crate) fn apply_exclusions(
    current: &RwLock<ExclusionSet>,
    local: Option<&Path>,
    backing: &Path,
    exclusions: ExclusionSet,
) {
    *current.write().expect("exclusions lock") = exclusions;
    let Some(local) = local else {
        return;
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
        gate: Arc<Gate>,
    ) -> Self {
        Self {
            backing,
            local,
            exclusions,
            inodes: Mutex::new(Inodes::default()),
            handles: Mutex::new(HashMap::new()),
            next_fh: AtomicU64::new(1),
            gate,
        }
    }

    fn inodes(&self) -> std::sync::MutexGuard<'_, Inodes> {
        self.inodes.lock().expect("inode table lock")
    }

    /// Waits for the backing folder; `EIO` if it does not come back.
    fn ready(&self) -> Result<(), Errno> {
        if self.gate.wait() {
            Ok(())
        } else {
            Err(Errno::EIO)
        }
    }

    /// Runs `op` on the backing folder once it is available.
    ///
    /// When rclone dies, its mount answers `ENOTCONN` or `ECONNABORTED`
    /// until the daemon notices (within a second), closes the gate and
    /// remounts. Such errors are retried until the gate's timeout instead
    /// of reaching the user.
    fn with_backing<T>(&self, mut op: impl FnMut() -> Result<T, Errno>) -> Result<T, Errno> {
        self.ready()?;
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

    /// Where `rel` lives: the local-only folder if it is excluded, else the
    /// backing folder (the hidden rclone mount).
    fn path_of(&self, rel: &Path) -> PathBuf {
        match &self.local {
            Some(local) if self.excluded(rel) => local.join(rel),
            _ => self.backing.join(rel),
        }
    }

    /// Like [`path_of`](Self::path_of), for an item about to be created:
    /// makes its folder exist in the local-only folder if needed.
    fn path_for_new(&self, rel: &Path) -> Result<PathBuf, Errno> {
        let path = self.path_of(rel);
        if !path.starts_with(&self.backing)
            && let Some(parent) = path.parent()
        {
            fs::create_dir_all(parent).map_err(errno)?;
        }
        Ok(path)
    }

    /// Absolute path of an inode.
    fn real(&self, ino: INodeNo) -> Result<PathBuf, Errno> {
        Ok(self.path_of(&self.rel(ino)?))
    }

    /// Relative path of `name` in directory `parent`.
    fn child(&self, parent: INodeNo, name: &OsStr) -> Result<PathBuf, Errno> {
        if name.as_encoded_bytes().contains(&b'/') || name == "." || name == ".." {
            return Err(Errno::EINVAL);
        }
        Ok(self.rel(parent)?.join(name))
    }

    /// Stats `rel` and counts a kernel lookup of it.
    fn entry(&self, rel: &Path) -> Result<FileAttr, Errno> {
        let meta = fs::symlink_metadata(self.path_of(rel)).map_err(errno)?;
        let ino = self.inodes().lookup(rel);
        Ok(attr(ino, &meta))
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
        mut op: impl FnMut(&File) -> io::Result<T>,
    ) -> Result<T, Errno> {
        let handle = self.file(fh)?;
        let Handle::File(of) = &*handle else {
            unreachable!("checked by file()")
        };
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
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let result = self.with_backing(|| self.entry(&self.child(parent, name)?));
        match result {
            Ok(attr) => reply.entry(&TTL, &attr, Generation(0)),
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
                return of.get().metadata().map_err(errno);
            }
            match self.real(ino) {
                Ok(path) => fs::symlink_metadata(path).map_err(errno),
                // Deleted while open: the kernel may not pass the handle.
                Err(e) => self.open_file_of(ino.0).ok_or(e)?.metadata().map_err(errno),
            }
        });
        match result {
            Ok(meta) => reply.attr(&TTL, &attr(ino.0, &meta)),
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
                .create(self.path_for_new(&rel)?)
                .map_err(errno)?;
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
            fs::remove_file(self.path_of(&rel)).map_err(errno)?;
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
            // Excluded items kept locally inside it count as content.
            let scaffold = self
                .local
                .as_ref()
                .filter(|_| !self.excluded(&rel))
                .map(|local| local.join(&rel));
            if let Some(dir) = &scaffold
                && fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
            {
                return Err(Errno::ENOTEMPTY);
            }
            fs::remove_dir(self.path_of(&rel)).map_err(errno)?;
            if let Some(dir) = scaffold {
                let _ = fs::remove_dir(dir);
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
            std::os::unix::fs::symlink(target, self.path_for_new(&rel)?).map_err(errno)?;
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
            let src = self.path_of(&from);
            let dst = self.path_for_new(&to)?;
            if src.starts_with(&self.backing) == dst.starts_with(&self.backing) {
                rustix::fs::renameat_with(rustix::fs::CWD, &src, rustix::fs::CWD, &dst, rflags)
                    .map_err(|e| Errno::from_i32(e.raw_os_error()))?;
                // Excluded items kept locally inside a moved folder follow it.
                if let Some(local) = &self.local
                    && src.starts_with(&self.backing)
                    && local.join(&from).is_dir()
                {
                    let target = local.join(&to);
                    if let Some(parent) = target.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::rename(local.join(&from), target);
                }
            } else {
                // Between the local-only folder and rclone: a temporary file
                // saved under its final name, or the reverse.
                self.move_across(&src, &dst, rflags)?;
            }
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
            let from = self.real(ino)?;
            let to = self.path_for_new(&rel)?;
            if from.starts_with(&self.backing) != to.starts_with(&self.backing) {
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
            let file = open_options(flags.0).open(self.real(ino)?).map_err(errno)?;
            Ok(self.add_handle(Handle::File(OpenFile::new(file, ino.0, flags.0))))
        });
        match result {
            Ok(fh) => reply.opened(fh, FopenFlags::empty()),
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
        let result = self.on_file(fh, |file| file.write_all_at(data, offset));
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
        self.handles
            .lock()
            .expect("handle table lock")
            .remove(&fh.0);
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
            // The folder itself, then (unless it is excluded) the excluded
            // items kept locally inside it.
            let mut sources = vec![self.path_of(&rel)];
            if let Some(local) = &self.local
                && !self.excluded(&rel)
            {
                sources.push(local.join(&rel));
            }
            let mut seen = std::collections::HashSet::new();
            let inodes = self.inodes();
            for (i, dir) in sources.iter().enumerate() {
                let entries = match fs::read_dir(dir) {
                    Ok(entries) => entries,
                    // No local item in this folder.
                    Err(_) if i > 0 => continue,
                    Err(e) => return Err(errno(e)),
                };
                let from_local = !dir.starts_with(&self.backing);
                for entry in entries {
                    let entry = entry.map_err(errno)?;
                    let name = entry.file_name();
                    let child = rel.join(&name);
                    // From rclone: not the excluded ones. Locally: only them.
                    if self.excluded(&child) != from_local || !seen.insert(name.clone()) {
                        continue;
                    }
                    let kind = entry
                        .file_type()
                        .ok()
                        .and_then(FileType::from_std)
                        .unwrap_or(FileType::RegularFile);
                    // Known inodes keep their number; others show the backing
                    // one (informational only: the kernel looks names up).
                    let ino = inodes.peek(&child).unwrap_or(entry.ino());
                    items.push(DirItem { ino, kind, name });
                }
            }
            drop(inodes);
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
            rustix::fs::statvfs(&self.backing).map_err(|e| Errno::from_i32(e.raw_os_error()))
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
            rustix::fs::access(self.real(ino)?, access)
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
            let file = options.open(self.path_for_new(&rel)?).map_err(errno)?;
            let meta = file.metadata().map_err(errno)?;
            let ino = self.inodes().lookup(&rel);
            // Reopened without O_CREAT/O_EXCL, but writable like here.
            let reopen = (flags & !libc::O_ACCMODE) | libc::O_RDWR;
            let fh = self.add_handle(Handle::File(OpenFile::new(file, ino, reopen)));
            Ok((attr(ino, &meta), fh))
        });
        match result {
            Ok((attr, fh)) => reply.created(&TTL, &attr, Generation(0), fh, FopenFlags::empty()),
            Err(e) => reply.error(e),
        }
    }
}
