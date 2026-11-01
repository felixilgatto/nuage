//! Startup checks of the `nuaged` binary that need no rclone.
//!
//! This integration test also makes `cargo test` build the `nuaged` binary
//! used by the end-to-end tests of the `nuage` crate.

use std::process::Command;

#[test]
fn missing_rclone_is_reported() {
    let dir = tempfile::Builder::new()
        .prefix("nuaged-")
        .tempdir_in("/tmp")
        .unwrap();
    let home = dir.path().join("home");
    let socket = dir.path().join("nuaged.sock");
    let out = Command::new(env!("CARGO_BIN_EXE_nuaged"))
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("NUAGE_SOCKET", &socket)
        .env("NUAGE_RCLONE", dir.path().join("no-rclone"))
        .env("NUAGE_KEYRING", "off")
        .env_remove("FLATPAK_ID")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cannot start rclone"), "{stderr}");
    assert!(stderr.contains("no-rclone"), "{stderr}");
    assert!(!socket.exists(), "the socket is removed on exit");
}
