//! Filesystem test suite of `nuage-fs`, mounted over a plain folder.
//!
//! Needs `/dev/fuse` and `fusermount3`; skipped without them unless
//! `NUAGE_E2E_REQUIRED=1` (set in CI).

use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use nuage_core::{ExclusionRule, ExclusionSet};
use nuage_fs::{MountConfig, MountedFs};

fn globs(patterns: &[&str]) -> ExclusionSet {
    let rules: Vec<ExclusionRule> = patterns
        .iter()
        .map(|p| ExclusionRule::Glob((*p).to_owned()))
        .collect();
    ExclusionSet::new(&rules)
}

fn with_exclusions(patterns: &[&str]) -> MountConfig {
    MountConfig {
        // The fixture puts the local-only folder next to the backing one.
        local_only: Some(PathBuf::new()),
        exclusions: globs(patterns),
        ..MountConfig::default()
    }
}

fn fuse_available() -> Result<(), String> {
    if !Path::new("/dev/fuse").exists() {
        return Err("/dev/fuse is missing".into());
    }
    let found = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .any(|d| d.join("fusermount3").is_file());
    if !found {
        return Err("fusermount3 is missing (install fuse3)".into());
    }
    Ok(())
}

struct Fixture {
    fs: Option<MountedFs>,
    backing: PathBuf,
    mnt: PathBuf,
    local: PathBuf,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new(config: MountConfig) -> Option<Self> {
        if let Err(reason) = fuse_available() {
            if std::env::var_os("NUAGE_E2E_REQUIRED").is_some() {
                panic!("filesystem test cannot run: {reason}");
            }
            eprintln!("skipping filesystem test: {reason}");
            return None;
        }
        let dir = tempfile::Builder::new()
            .prefix("nuage-fs-")
            .tempdir_in("/tmp")
            .unwrap();
        let backing = dir.path().join("backing");
        let mnt = dir.path().join("mnt");
        let local = dir.path().join("local-only");
        fs::create_dir_all(&backing).unwrap();
        fs::create_dir_all(&mnt).unwrap();
        let config = MountConfig {
            local_only: config.local_only.map(|_| local.clone()),
            ..config
        };
        let fs = nuage_fs::mount(&backing, &mnt, &config).unwrap();
        Some(Self {
            fs: Some(fs),
            backing,
            mnt,
            local,
            _dir: dir,
        })
    }

    fn m(&self, rel: &str) -> PathBuf {
        self.mnt.join(rel)
    }

    fn b(&self, rel: &str) -> PathBuf {
        self.backing.join(rel)
    }

    fn l(&self, rel: &str) -> PathBuf {
        self.local.join(rel)
    }

    fn fs(&self) -> &MountedFs {
        self.fs.as_ref().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(fs) = self.fs.take() {
            fs.unmount().unwrap();
        }
    }
}

macro_rules! fixture {
    () => {
        fixture!(MountConfig::default())
    };
    ($config:expr) => {
        match Fixture::new($config) {
            Some(f) => f,
            None => return,
        }
    };
}

fn is_mounted(path: &Path) -> bool {
    let info = fs::read_to_string("/proc/self/mountinfo").unwrap();
    info.lines()
        .any(|l| l.split(' ').nth(4) == Some(path.to_str().unwrap()))
}

#[test]
fn mount_and_unmount() {
    let mut f = fixture!();
    assert!(is_mounted(&f.mnt));
    let info = fs::read_to_string("/proc/self/mountinfo").unwrap();
    assert!(
        info.contains("fuse.nuage"),
        "subtype shown in the mount table"
    );
    f.fs.take().unwrap().unmount().unwrap();
    assert!(!is_mounted(&f.mnt));
}

#[test]
fn read_write() {
    let f = fixture!();
    fs::write(f.b("existing.txt"), "from the backing folder").unwrap();
    assert_eq!(
        fs::read_to_string(f.m("existing.txt")).unwrap(),
        "from the backing folder"
    );

    fs::write(f.m("new.txt"), "hello").unwrap();
    assert_eq!(fs::read_to_string(f.b("new.txt")).unwrap(), "hello");

    // Append.
    let mut file = OpenOptions::new()
        .append(true)
        .open(f.m("new.txt"))
        .unwrap();
    file.write_all(b" world").unwrap();
    drop(file);
    assert_eq!(fs::read_to_string(f.m("new.txt")).unwrap(), "hello world");

    // Overwrite in the middle, read at offsets.
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.m("new.txt"))
        .unwrap();
    file.write_all_at(b"HELLO", 0).unwrap();
    let mut buf = [0u8; 5];
    file.read_exact_at(&mut buf, 6).unwrap();
    assert_eq!(&buf, b"world");
    drop(file);
    assert_eq!(fs::read_to_string(f.b("new.txt")).unwrap(), "HELLO world");

    // O_TRUNC and set_len.
    fs::write(f.m("new.txt"), "short").unwrap();
    assert_eq!(fs::read_to_string(f.m("new.txt")).unwrap(), "short");
    let file = OpenOptions::new().write(true).open(f.m("new.txt")).unwrap();
    file.set_len(2).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 2);
    file.set_len(10).unwrap();
    drop(file);
    assert_eq!(fs::read(f.m("new.txt")).unwrap(), b"sh\0\0\0\0\0\0\0\0");

    // Exclusive creation.
    let err = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(f.m("new.txt"))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::AlreadyExists);

    // A file created read-only.
    let file = OpenOptions::new()
        .read(true)
        .write(false)
        .create(false)
        .open(f.m("new.txt"));
    assert!(file.is_ok());
}

#[test]
fn large_file_and_seeks() {
    let f = fixture!();
    let data: Vec<u8> = (0..20 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    fs::write(f.m("big.bin"), &data).unwrap();
    assert_eq!(fs::read(f.b("big.bin")).unwrap(), data);
    assert_eq!(fs::read(f.m("big.bin")).unwrap(), data);

    let mut file = File::open(f.m("big.bin")).unwrap();
    for offset in [0u64, 1, 4095, 4096, 1 << 20, (20 << 20) - 3] {
        file.seek(SeekFrom::Start(offset)).unwrap();
        let mut buf = [0u8; 3];
        let n = file.read(&mut buf).unwrap();
        assert!(n > 0);
        for (i, b) in buf[..n].iter().enumerate() {
            assert_eq!(*b, data[offset as usize + i], "offset {offset}");
        }
    }
    // Reading past the end returns nothing.
    file.seek(SeekFrom::Start(30 << 20)).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(file.read(&mut buf).unwrap(), 0);
}

#[test]
fn directories() {
    let f = fixture!();
    fs::create_dir_all(f.m("a/b/c")).unwrap();
    assert!(f.b("a/b/c").is_dir());
    assert_eq!(
        fs::create_dir(f.m("a")).unwrap_err().kind(),
        ErrorKind::AlreadyExists
    );

    for i in 0..1000 {
        fs::write(f.m(&format!("a/file-{i:04}")), i.to_string()).unwrap();
    }
    let mut names: Vec<String> = fs::read_dir(f.m("a"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names.len(), 1001);
    assert_eq!(names[0], "b");
    assert_eq!(names[1000], "file-0999");
    // Listing types without stat.
    let kinds: Vec<bool> = fs::read_dir(f.m("a/b"))
        .unwrap()
        .map(|e| e.unwrap().file_type().unwrap().is_dir())
        .collect();
    assert_eq!(kinds, vec![true]);

    let err = fs::remove_dir(f.m("a/b")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOTEMPTY));
    fs::remove_dir(f.m("a/b/c")).unwrap();
    fs::remove_dir(f.m("a/b")).unwrap();
    assert!(!f.b("a/b").exists());

    let err = fs::read_dir(f.m("a/file-0001")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOTDIR));
    let err = fs::metadata(f.m("nope")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn rename() {
    let f = fixture!();
    fs::write(f.m("a.txt"), "a").unwrap();
    fs::rename(f.m("a.txt"), f.m("b.txt")).unwrap();
    assert!(!f.m("a.txt").exists());
    assert_eq!(fs::read_to_string(f.m("b.txt")).unwrap(), "a");
    assert!(f.b("b.txt").exists());

    // Overwrite an existing file.
    fs::write(f.m("c.txt"), "c").unwrap();
    fs::rename(f.m("b.txt"), f.m("c.txt")).unwrap();
    assert_eq!(fs::read_to_string(f.m("c.txt")).unwrap(), "a");

    // A directory with content, then its children by their new path.
    fs::create_dir_all(f.m("dir/sub")).unwrap();
    fs::write(f.m("dir/sub/x"), "x").unwrap();
    let _ = fs::metadata(f.m("dir/sub/x")).unwrap();
    fs::rename(f.m("dir"), f.m("moved")).unwrap();
    assert_eq!(fs::read_to_string(f.m("moved/sub/x")).unwrap(), "x");
    assert!(!f.m("dir").exists());
    fs::write(f.m("moved/sub/y"), "y").unwrap();
    assert_eq!(fs::read_to_string(f.b("moved/sub/y")).unwrap(), "y");

    // An open file keeps working after its folder was renamed.
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.m("moved/sub/x"))
        .unwrap();
    fs::rename(f.m("moved"), f.m("again")).unwrap();
    file.write_all(b"X").unwrap();
    drop(file);
    assert_eq!(fs::read_to_string(f.m("again/sub/x")).unwrap(), "X");

    // RENAME_NOREPLACE and RENAME_EXCHANGE.
    fs::write(f.m("p"), "p").unwrap();
    fs::write(f.m("q"), "q").unwrap();
    let err = rustix::fs::renameat_with(
        rustix::fs::CWD,
        f.m("p"),
        rustix::fs::CWD,
        f.m("q"),
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .unwrap_err();
    assert_eq!(err, rustix::io::Errno::EXIST);
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        f.m("p"),
        rustix::fs::CWD,
        f.m("q"),
        rustix::fs::RenameFlags::EXCHANGE,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(f.m("p")).unwrap(), "q");
    assert_eq!(fs::read_to_string(f.m("q")).unwrap(), "p");

    // Renaming into a missing folder fails cleanly.
    let err = fs::rename(f.m("p"), f.m("missing/p")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn delete() {
    let f = fixture!();
    fs::write(f.m("gone.txt"), "still readable").unwrap();
    let mut open = File::open(f.m("gone.txt")).unwrap();
    fs::remove_file(f.m("gone.txt")).unwrap();
    assert!(!f.m("gone.txt").exists());
    assert!(!f.b("gone.txt").exists());
    // Deleted while open: the handle still reads and stats.
    let mut s = String::new();
    open.read_to_string(&mut s).unwrap();
    assert_eq!(s, "still readable");
    assert_eq!(open.metadata().unwrap().len(), 14);
    // A new file with the same name is a different file.
    fs::write(f.m("gone.txt"), "new").unwrap();
    assert_eq!(fs::read_to_string(f.m("gone.txt")).unwrap(), "new");

    assert_eq!(
        fs::remove_file(f.m("gone-twice")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn attributes_and_links() {
    let f = fixture!();
    fs::write(f.m("file"), "12345").unwrap();
    let meta = fs::metadata(f.m("file")).unwrap();
    assert_eq!(meta.len(), 5);
    assert!(meta.is_file());
    assert_eq!(meta.uid(), fs::metadata(&f.backing).unwrap().uid());

    fs::set_permissions(f.m("file"), fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(fs::metadata(f.b("file")).unwrap().mode() & 0o777, 0o640);
    assert_eq!(fs::metadata(f.m("file")).unwrap().mode() & 0o777, 0o640);

    let when = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    File::options()
        .write(true)
        .open(f.m("file"))
        .unwrap()
        .set_modified(when)
        .unwrap();
    assert_eq!(fs::metadata(f.b("file")).unwrap().modified().unwrap(), when);
    // Directories too.
    fs::create_dir(f.m("d")).unwrap();
    File::open(f.m("d")).unwrap().set_modified(when).unwrap();
    assert_eq!(fs::metadata(f.b("d")).unwrap().modified().unwrap(), when);

    std::os::unix::fs::symlink("file", f.m("link")).unwrap();
    assert_eq!(fs::read_link(f.m("link")).unwrap(), Path::new("file"));
    assert_eq!(fs::read_to_string(f.m("link")).unwrap(), "12345");
    fs::hard_link(f.m("file"), f.m("hard")).unwrap();
    assert_eq!(fs::metadata(f.m("hard")).unwrap().nlink(), 2);

    let stat = rustix::fs::statvfs(&f.mnt).unwrap();
    assert!(stat.f_blocks > 0);
    assert_eq!(
        stat.f_blocks,
        rustix::fs::statvfs(&f.backing).unwrap().f_blocks
    );

    rustix::fs::access(f.m("file"), rustix::fs::Access::READ_OK).unwrap();
}

#[test]
fn concurrent_access() {
    let f = fixture!();
    let threads: Vec<_> = (0..8)
        .map(|t| {
            let dir = f.mnt.clone();
            std::thread::spawn(move || {
                for i in 0..50 {
                    let path = dir.join(format!("t{t}-{i}"));
                    let data = format!("{t}:{i}").repeat(1000);
                    fs::write(&path, &data).unwrap();
                    assert_eq!(fs::read_to_string(&path).unwrap(), data);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(fs::read_dir(&f.mnt).unwrap().count(), 400);
}

#[test]
fn waits_while_unavailable() {
    let f = fixture!(MountConfig {
        wait: Duration::from_secs(10),
        ..MountConfig::default()
    });
    fs::write(f.b("file"), "x").unwrap();
    f.fs().set_available(false);
    let path = f.m("file");
    let start = Instant::now();
    let reader = std::thread::spawn(move || fs::read_to_string(path));
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !reader.is_finished(),
        "requests wait for the backing folder"
    );
    f.fs().set_available(true);
    assert_eq!(reader.join().unwrap().unwrap(), "x");
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn fails_cleanly_when_unavailable_too_long() {
    let f = fixture!(MountConfig {
        wait: Duration::from_millis(300),
        ..MountConfig::default()
    });
    f.fs().set_available(false);
    let err = fs::read_dir(f.m("")).map(|_| ()).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::EIO));
    f.fs().set_available(true);
    fs::write(f.m("ok"), "ok").unwrap();
}

#[test]
fn excluded_items_stay_local() {
    let f = fixture!(with_exclusions(&[
        "*.tmp",
        "node_modules/",
        ".goutputstream-*"
    ]));
    // Excluded items already in the cloud are hidden.
    fs::write(f.b("remote.tmp"), "remote").unwrap();
    fs::write(f.b("kept.txt"), "kept").unwrap();
    let names = |dir: &Path| -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(&f.mnt), vec!["kept.txt"]);
    assert_eq!(
        fs::metadata(f.m("remote.tmp")).unwrap_err().kind(),
        ErrorKind::NotFound
    );

    // Created locally: kept local, visible at its path, never in the backing folder.
    fs::write(f.m("draft.tmp"), "draft").unwrap();
    assert!(f.l("draft.tmp").is_file());
    assert!(
        !f.b("draft.tmp").exists() || fs::read_to_string(f.b("draft.tmp")).unwrap() == "remote"
    );
    assert_eq!(fs::read_to_string(f.m("draft.tmp")).unwrap(), "draft");
    fs::create_dir_all(f.m("proj/node_modules/lib")).unwrap();
    fs::write(f.m("proj/node_modules/lib/index.js"), "js").unwrap();
    fs::write(f.m("proj/main.js"), "main").unwrap();
    assert!(f.b("proj/main.js").is_file());
    assert!(!f.b("proj/node_modules").exists());
    assert!(f.l("proj/node_modules/lib/index.js").is_file());
    assert_eq!(names(&f.m("proj")), vec!["main.js", "node_modules"]);
    assert_eq!(names(&f.m("proj/node_modules/lib")), vec!["index.js"]);

    // A folder holding local items is not empty.
    fs::remove_file(f.m("proj/main.js")).unwrap();
    let err = fs::remove_dir(f.m("proj")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ENOTEMPTY));
    fs::remove_dir_all(f.m("proj/node_modules")).unwrap();
    fs::remove_dir(f.m("proj")).unwrap();
    assert!(!f.b("proj").exists());
    assert!(!f.l("proj").exists());

    // GNOME's atomic save: a temporary file renamed to its final name.
    fs::write(f.m(".goutputstream-AB12"), "saved").unwrap();
    assert!(!f.b(".goutputstream-AB12").exists());
    fs::rename(f.m(".goutputstream-AB12"), f.m("notes.txt")).unwrap();
    assert_eq!(fs::read_to_string(f.b("notes.txt")).unwrap(), "saved");
    assert!(!f.l(".goutputstream-AB12").exists());
    assert_eq!(fs::read_to_string(f.m("notes.txt")).unwrap(), "saved");
    // And the reverse: renamed to an excluded name, it leaves the cloud.
    fs::rename(f.m("notes.txt"), f.m("notes.tmp")).unwrap();
    assert!(!f.b("notes.txt").exists());
    assert_eq!(fs::read_to_string(f.l("notes.tmp")).unwrap(), "saved");
}

#[test]
fn reincluded_items_go_to_the_cloud() {
    let f = fixture!(with_exclusions(&["*.tmp", "build/"]));
    fs::write(f.m("a.tmp"), "a").unwrap();
    fs::create_dir(f.m("build")).unwrap();
    fs::write(f.m("build/out.bin"), "bin").unwrap();
    assert!(!f.b("a.tmp").exists());

    // `*.tmp` is no longer excluded: a.tmp moves to the backing folder.
    f.fs().set_exclusions(globs(&["build/"]));
    assert_eq!(fs::read_to_string(f.b("a.tmp")).unwrap(), "a");
    assert!(!f.l("a.tmp").exists());
    assert_eq!(fs::read_to_string(f.m("a.tmp")).unwrap(), "a");
    assert!(f.l("build/out.bin").is_file());

    // A new exclusion hides what is already in the cloud.
    f.fs().set_exclusions(globs(&["build/", "a.tmp"]));
    assert_eq!(
        fs::metadata(f.m("a.tmp")).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert!(f.b("a.tmp").exists(), "nothing is deleted in the cloud");
}
