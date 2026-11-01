//! Mounts a real FUSE filesystem (an rclone mount) through the relay.
//!
//! A `fusermount3` placed first in `PATH` leaves a marker and runs the relay,
//! as the Flatpak wrapper does through `flatpak-spawn --host`. Needs a
//! non-snap rclone (`NUAGE_RCLONE` or `PATH`), `/dev/fuse` and the host's
//! `fusermount3`; skipped without them unless `NUAGE_E2E_REQUIRED=1`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

fn requirements() -> Result<PathBuf, String> {
    let rclone = match std::env::var_os("NUAGE_RCLONE") {
        Some(p) => PathBuf::from(p),
        None => find_in_path("rclone").ok_or("rclone not found (set NUAGE_RCLONE)")?,
    };
    if rclone.starts_with("/snap") {
        return Err("the rclone snap cannot mount (set NUAGE_RCLONE)".into());
    }
    if !Path::new("/dev/fuse").exists() {
        return Err("/dev/fuse is missing".into());
    }
    find_in_path("fusermount3").ok_or("fusermount3 is missing (install fuse3)")?;
    Ok(rclone)
}

fn mounted(path: &Path) -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .unwrap()
        .lines()
        .any(|l| l.split(' ').nth(4) == Some(path.to_str().unwrap()))
}

#[test]
fn rclone_mounts_through_the_relay() {
    let rclone = match requirements() {
        Ok(r) => r,
        Err(reason) => {
            if std::env::var_os("NUAGE_E2E_REQUIRED").is_some() {
                panic!("relay test cannot run: {reason}");
            }
            eprintln!("skipping relay test: {reason}");
            return;
        }
    };
    let dir = tempfile::Builder::new()
        .prefix("nuage-relay-")
        .tempdir_in("/tmp")
        .unwrap();
    let (bin, src, mnt) = (
        dir.path().join("bin"),
        dir.path().join("src"),
        dir.path().join("mnt"),
    );
    for d in [&bin, &src, &mnt] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(src.join("hello.txt"), "hello").unwrap();

    // The stand-in for the Flatpak wrapper.
    let marker = dir.path().join("relay-used");
    let wrapper = bin.join("fusermount3");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\n[ -n \"$_FUSE_COMMFD\" ] && touch '{}'\nNUAGE_HOST_FUSERMOUNT='{}' exec '{}' \"$@\"\n",
            marker.display(),
            find_in_path("fusermount3").unwrap().display(),
            env!("CARGO_BIN_EXE_nuage-fuse-relay")
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut child = Command::new(&rclone)
        .arg("mount")
        .arg(&src)
        .arg(&mnt)
        .arg("--config")
        .arg(dir.path().join("rclone.conf"))
        .arg("--cache-dir")
        .arg(dir.path().join("cache"))
        .env("PATH", &path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while !mounted(&mnt) {
        if let Some(status) = child.try_wait().unwrap() {
            let out = child.wait_with_output().unwrap();
            panic!(
                "rclone exited ({status}): {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(Instant::now() < deadline, "the mount did not appear");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(marker.exists(), "the mount went through the relay");
    assert_eq!(
        std::fs::read_to_string(mnt.join("hello.txt")).unwrap(),
        "hello"
    );
    std::fs::write(mnt.join("new.txt"), "new").unwrap();

    // Unmount through the relay too (no descriptor to pass).
    let status = Command::new(&wrapper)
        .args(["-u"])
        .arg(&mnt)
        .env("PATH", &path)
        .status()
        .unwrap();
    assert!(status.success());
    let _ = child.wait();
    assert!(!mounted(&mnt));
    assert_eq!(std::fs::read_to_string(src.join("new.txt")).unwrap(), "new");
}

#[test]
fn bad_descriptor_is_refused() {
    let out = Command::new(env!("CARGO_BIN_EXE_nuage-fuse-relay"))
        .env("_FUSE_COMMFD", "nope")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("invalid _FUSE_COMMFD"));

    let out = Command::new(env!("CARGO_BIN_EXE_nuage-fuse-relay"))
        .env("_FUSE_COMMFD", "200")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not an open descriptor"));
}

#[test]
fn loops_are_refused() {
    let out = Command::new(env!("CARGO_BIN_EXE_nuage-fuse-relay"))
        .arg("--version")
        .env("NUAGE_FUSE_RELAY_ACTIVE", "1")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("called by itself"));
}
