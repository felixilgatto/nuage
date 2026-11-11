//! End-to-end tests: the real `nuaged` and `nuage` binaries, a real rclone
//! and FUSE, with the `local` provider.
//!
//! Needs a non-snap rclone (`NUAGE_RCLONE` or `rclone` in `PATH`),
//! `/dev/fuse` and `fusermount3`. Without them the tests are skipped, unless
//! `NUAGE_E2E_REQUIRED=1` (set in CI), which turns a skip into a failure.
//! `nuaged` must be built and up to date: `cargo test --workspace`, or
//! `cargo test -p nuaged -p nuage` (`-p nuage` alone does not rebuild it).
//! `NUAGE_TEST_NUAGED` points the tests at another `nuaged` binary.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

/// Sign-ins use rclone's fixed port 53682: one test at a time.
static SIGN_IN_PORT: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn find_in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

/// rclone able to mount, or the reason why there is none.
fn usable_rclone() -> Result<PathBuf, String> {
    let rclone = match std::env::var_os("NUAGE_RCLONE") {
        Some(p) => PathBuf::from(p),
        None => find_in_path("rclone").ok_or("rclone not found (set NUAGE_RCLONE)")?,
    };
    if rclone.starts_with("/snap") {
        return Err(format!(
            "{} is a snap, which cannot mount (set NUAGE_RCLONE)",
            rclone.display()
        ));
    }
    if !Path::new("/dev/fuse").exists() {
        return Err("/dev/fuse is missing".into());
    }
    if find_in_path("fusermount3").is_none() {
        return Err("fusermount3 is missing (install fuse3)".into());
    }
    Ok(rclone)
}

fn nuaged_binary() -> PathBuf {
    // Packaged binaries (e.g. the static build) can be tested too.
    if let Some(path) = std::env::var_os("NUAGE_TEST_NUAGED") {
        return PathBuf::from(path);
    }
    let nuage = PathBuf::from(env!("CARGO_BIN_EXE_nuage"));
    let nuaged = nuage.with_file_name("nuaged");
    assert!(
        nuaged.exists(),
        "{} is missing: build it with `cargo build -p nuaged`",
        nuaged.display()
    );
    nuaged
}

/// Skips the test when rclone or FUSE is missing.
macro_rules! env_or_skip {
    () => {
        match usable_rclone() {
            Ok(rclone) => TestEnv::new(rclone),
            Err(reason) => {
                if std::env::var_os("NUAGE_E2E_REQUIRED").is_some() {
                    panic!("end-to-end test cannot run: {reason}");
                }
                eprintln!("skipping end-to-end test: {reason}");
                return;
            }
        }
    };
}

/// An isolated home, daemon socket and rclone configuration.
struct TestEnv {
    // Dropped last, after the daemon is stopped and the mounts are gone.
    dir: tempfile::TempDir,
    rclone: PathBuf,
    nuaged: PathBuf,
    daemon: Option<Child>,
    /// More environment for the daemon and the CLI.
    extra_env: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

impl TestEnv {
    fn new(rclone: PathBuf) -> Self {
        // Short path: Unix socket paths are limited to 107 bytes.
        let dir = tempfile::Builder::new()
            .prefix("nuage-e2e-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::create_dir_all(dir.path().join("home")).unwrap();
        Self {
            dir,
            rclone,
            nuaged: nuaged_binary(),
            daemon: None,
            extra_env: Vec::new(),
        }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn socket(&self) -> PathBuf {
        self.path("nuaged.sock")
    }

    fn apply_env(&self, cmd: &mut Command) {
        let home = self.path("home");
        cmd.env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("XDG_CACHE_HOME", home.join(".cache"))
            .env("XDG_DATA_HOME", home.join(".local/share"))
            .env("NUAGE_SOCKET", self.socket())
            .env("NUAGE_RCLONE", &self.rclone)
            .env("NUAGE_LOG", "debug")
            // Never touch the keyring of the user running the tests.
            .env("NUAGE_KEYRING", "off")
            .env_remove("FLATPAK_ID");
        for (k, v) in &self.extra_env {
            cmd.env(k, v);
        }
    }

    fn start_daemon(&mut self) {
        assert!(self.daemon.is_none(), "daemon already started");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.path("daemon.log"))
            .unwrap();
        let mut cmd = Command::new(&self.nuaged);
        self.apply_env(&mut cmd);
        let child = cmd
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        self.daemon = Some(child);
        self.wait_until("the daemon answers", Duration::from_secs(20), |env| {
            env.nuage(&["daemon", "status"]).status.success()
        });
    }

    /// `nuage daemon stop --now`, then waits for the process to exit.
    fn stop_daemon(&mut self) {
        let out = self.nuage(&["daemon", "stop", "--now"]);
        assert!(out.status.success(), "{}", self.describe(&out));
        let mut child = self.daemon.take().expect("daemon running");
        let deadline = Instant::now() + Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "the daemon did not stop\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// `SIGKILL`: no cleanup at all, like a crash.
    fn kill_daemon(&mut self) {
        let mut child = self.daemon.take().expect("daemon running");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    fn nuage(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_nuage"));
        self.apply_env(&mut cmd);
        cmd.args(args).stdin(Stdio::null()).output().unwrap()
    }

    /// Runs `nuage --json …`, checks it succeeded and parses its output.
    fn json(&self, args: &[&str]) -> Value {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.nuage(&all);
        assert!(
            out.status.success(),
            "nuage {args:?} failed\n{}",
            self.describe(&out)
        );
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
            panic!(
                "nuage {args:?}: invalid JSON ({e})\n{}",
                self.describe(&out)
            )
        })
    }

    /// Runs `nuage --json …`, checks it failed with `exit` and returns the
    /// error code.
    fn json_error(&self, args: &[&str], exit: i32) -> String {
        let mut all = vec!["--json"];
        all.extend_from_slice(args);
        let out = self.nuage(&all);
        assert_eq!(
            out.status.code(),
            Some(exit),
            "nuage {args:?}\n{}",
            self.describe(&out)
        );
        let v: Value = serde_json::from_slice(&out.stdout).unwrap();
        v["error"]["code"].as_str().unwrap().to_owned()
    }

    fn status(&self) -> Value {
        self.json(&["status"])
    }

    fn account_state(&self, name: &str) -> Value {
        let status = self.status();
        status["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["account"]["display_name"] == name)
            .unwrap_or_else(|| panic!("no account {name} in {status}"))
            .clone()
    }

    /// Creates a source folder and adds a `local` account over it.
    fn add_local(&self, name: &str) -> (PathBuf, PathBuf) {
        let src = self.path(&format!("src-{name}"));
        std::fs::create_dir_all(&src).unwrap();
        let mount = self.path(&format!("home/Cloud/{name}"));
        let account = self.json(&[
            "account",
            "add",
            "local",
            "--name",
            name,
            "--mount-point",
            mount.to_str().unwrap(),
            "--param",
            &format!("root={}", src.display()),
        ]);
        assert_eq!(account["display_name"], name);
        (src, mount)
    }

    /// Waits for the daemon to have mounted `name` at `mount`: since it
    /// answers clients before its startup mounts are done, right after a
    /// start they may still be in progress.
    fn wait_mounted(&self, name: &str, mount: &Path) {
        self.wait_until(
            &format!("\"{name}\" is mounted"),
            Duration::from_secs(20),
            |env| env.account_state(name)["mounted"] == true && fuse_mounted(mount),
        );
    }

    fn wait_until(&self, what: &str, timeout: Duration, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + timeout;
        while !done(self) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting until {what}\n{}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    fn log(&self) -> String {
        let log = std::fs::read_to_string(self.path("daemon.log")).unwrap_or_default();
        // The end of the log is what matters.
        let start = log.len().saturating_sub(20_000);
        let start = (start..log.len())
            .find(|&i| log.is_char_boundary(i))
            .unwrap_or(0);
        format!("--- daemon log ---\n{}", &log[start..])
    }

    fn describe(&self, out: &Output) -> String {
        format!(
            "status: {:?}\nstdout: {}\nstderr: {}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            self.log()
        )
    }
}

fn fuse_mounted(path: &Path) -> bool {
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    let wanted = path.to_str().unwrap().replace(' ', "\\040");
    info.lines().any(|l| {
        let fields: Vec<&str> = l.split(' ').collect();
        fields.get(4) == Some(&wanted.as_str()) && l.contains(" - fuse")
    })
}

/// Filesystem type of the mount at `path` (`fuse.nuage`, `fuse.rclone`…).
fn mount_type(path: &Path) -> Option<String> {
    let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap();
    let wanted = path.to_str().unwrap().replace(' ', "\\040");
    info.lines().find_map(|l| {
        let fields: Vec<&str> = l.split(' ').collect();
        if fields.get(4) != Some(&wanted.as_str()) {
            return None;
        }
        let sep = fields.iter().position(|f| *f == "-")?;
        fields.get(sep + 1).map(|t| t.to_string())
    })
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        if let Some(mut child) = self.daemon.take() {
            let _ = self.nuage(&["daemon", "stop", "--now"]);
            let deadline = Instant::now() + Duration::from_secs(10);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        // Never leave a mount behind, or the temporary directory cannot be
        // deleted.
        let info = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let root = self.dir.path().to_str().unwrap().to_owned();
        for line in info.lines() {
            if let Some(mp) = line.split(' ').nth(4)
                && mp.starts_with(&root)
            {
                let mp = mp.replace("\\040", " ");
                let _ = Command::new("fusermount3").args(["-u", "-z", &mp]).status();
            }
        }
        // A stray rcd from a killed daemon.
        if let Ok(pid) = std::fs::read_to_string(self.path("home/.local/share/nuage/rcd.pid")) {
            let _ = Command::new("kill").arg(pid.trim()).status();
        }
    }
}

#[test]
fn account_lifecycle() {
    let mut env = env_or_skip!();
    env.start_daemon();

    let info = env.json(&["daemon", "status"]);
    assert!(info["rclone_version"].as_str().unwrap().starts_with('v'));
    assert_eq!(env.status()["accounts"], Value::Array(vec![]));

    let src = env.path("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("hello.txt"), "hello").unwrap();
    // Default name and folder: the provider name, under ~/Cloud.
    let account = env.json(&[
        "account",
        "add",
        "local",
        "--param",
        &format!("root={}", src.display()),
    ]);
    assert_eq!(account["display_name"], "Local folder");
    let mount = env.path("home/Cloud/Local folder");
    assert_eq!(account["mount_point"], mount.to_str().unwrap());
    assert!(fuse_mounted(&mount));
    // nuage-fs at the user's folder, rclone at the hidden folder.
    assert_eq!(mount_type(&mount).as_deref(), Some("fuse.nuage"));
    let hidden = env.path(&format!(
        "home/.local/share/nuage/mounts/{}",
        account["id"].as_str().unwrap()
    ));
    assert_eq!(mount_type(&hidden).as_deref(), Some("fuse.rclone"));

    // Same provider again: suffixed name.
    let (_, other_mount) = (env.path("src2"), env.path("home/elsewhere"));
    std::fs::create_dir_all(env.path("src2")).unwrap();
    let second = env.json(&[
        "account",
        "add",
        "local",
        "--mount-point",
        other_mount.to_str().unwrap(),
        "--param",
        &format!("root={}", env.path("src2").display()),
    ]);
    assert_eq!(second["display_name"], "Local folder 2");

    // Files are readable and writable through the mount; writes reach the
    // source once released.
    assert_eq!(
        std::fs::read_to_string(mount.join("hello.txt")).unwrap(),
        "hello"
    );
    std::fs::write(mount.join("new.txt"), "new").unwrap();
    env.wait_until("new.txt is uploaded", Duration::from_secs(20), |_| {
        std::fs::read_to_string(src.join("new.txt")).is_ok_and(|s| s == "new")
    });
    env.wait_until(
        "the account is up to date",
        Duration::from_secs(20),
        |env| env.account_state("Local folder")["state"]["state"] == "up_to_date",
    );
    // Without a keyring, rclone.conf is plain but private.
    assert_eq!(env.status()["config_encrypted"], false);
    use std::os::unix::fs::PermissionsExt;
    let conf = std::fs::metadata(env.path("home/.config/nuage/rclone.conf")).unwrap();
    assert_eq!(conf.permissions().mode() & 0o777, 0o600);
    let status = env.account_state("Local folder");
    assert_eq!(status["mounted"], true);
    assert!(status["quota"]["total"].as_u64().unwrap() > 0, "{status}");

    // Names are case-insensitive on the command line.
    let renamed = env.json(&["account", "rename", "local folder", "Files"]);
    assert_eq!(renamed["display_name"], "Files");
    assert_eq!(
        env.json_error(&["account", "rename", "Files", "Local folder 2"], 1),
        "already_exists"
    );

    // Unmount stays unmounted across restarts; mount brings it back.
    env.json(&["unmount", "Files"]);
    assert!(!fuse_mounted(&mount));
    assert_eq!(env.account_state("Files")["state"]["state"], "stopped");
    env.stop_daemon();
    env.start_daemon();
    env.wait_mounted("Local folder 2", &other_mount);
    assert_eq!(env.account_state("Files")["mounted"], false);
    env.json(&["mount", "Files"]);
    assert!(fuse_mounted(&mount));
    assert_eq!(
        std::fs::read_to_string(mount.join("new.txt")).unwrap(),
        "new"
    );

    // Removal needs --yes, deletes local data only.
    assert_eq!(env.json_error(&["account", "remove", "Files"], 2), "usage");
    env.json(&["account", "remove", "Files", "--yes"]);
    assert!(!fuse_mounted(&mount));
    assert!(!mount.exists(), "the empty mount folder is removed");
    assert_eq!(
        std::fs::read_to_string(src.join("hello.txt")).unwrap(),
        "hello"
    );
    let accounts = env.json(&["account", "list"]);
    assert_eq!(accounts.as_array().unwrap().len(), 1);
    assert_eq!(env.json_error(&["mount", "Files"], 1), "not_found");
}

#[test]
fn pause_holds_uploads_across_restarts() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let (src, mount) = env.add_local("Work");

    env.json(&["pause", "Work"]);
    std::fs::write(mount.join("held.txt"), "held").unwrap();
    env.wait_until("the write is queued", Duration::from_secs(10), |env| {
        env.account_state("Work")["state"] == serde_json::json!({"state": "paused", "queued": 1})
    });
    std::thread::sleep(Duration::from_secs(4));
    assert!(
        !src.join("held.txt").exists(),
        "paused uploads must not leave"
    );
    // The file is usable locally meanwhile.
    assert_eq!(
        std::fs::read_to_string(mount.join("held.txt")).unwrap(),
        "held"
    );

    // The pause and the pending write survive a restart.
    env.stop_daemon();
    assert!(!fuse_mounted(&mount));
    env.start_daemon();
    assert_eq!(env.account_state("Work")["state"]["state"], "paused");
    std::thread::sleep(Duration::from_secs(3));
    assert!(!src.join("held.txt").exists());

    env.json(&["resume"]);
    env.wait_until("held.txt is uploaded", Duration::from_secs(20), |_| {
        std::fs::read_to_string(src.join("held.txt")).is_ok_and(|s| s == "held")
    });
    env.wait_until(
        "the account is up to date",
        Duration::from_secs(20),
        |env| env.account_state("Work")["state"]["state"] == "up_to_date",
    );
}

#[test]
fn restart_after_a_crash() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let (src, mount) = env.add_local("Crash");
    std::fs::write(src.join("a.txt"), "a").unwrap();

    env.kill_daemon();
    let stale_socket = env.socket();
    assert!(stale_socket.exists(), "a killed daemon leaves its socket");

    env.start_daemon();
    env.wait_mounted("Crash", &mount);
    assert_eq!(std::fs::read_to_string(mount.join("a.txt")).unwrap(), "a");
    // Only one rcd runs: the previous one was stopped.
    let rcds = Command::new("pgrep")
        .args(["-f", &format!("rclone rcd .*{}", env.dir.path().display())])
        .output()
        .unwrap();
    let count = String::from_utf8_lossy(&rcds.stdout).lines().count();
    assert!(count <= 1, "{count} rclone rcd processes");
}

#[test]
fn errors_and_settings() {
    let mut env = env_or_skip!();
    assert_eq!(env.json_error(&["status"], 3), "daemon_unreachable");
    env.start_daemon();

    assert_eq!(env.json_error(&["mount", "Nope"], 1), "not_found");
    assert_eq!(
        env.json_error(&["account", "add", "nope"], 2),
        "invalid_argument"
    );
    assert_eq!(
        env.json_error(&["account", "add", "local"], 2),
        "invalid_argument"
    );
    assert_eq!(
        env.json_error(&["account", "add", "gdrive", "--token", "{}"], 2),
        "invalid_argument"
    );
    let full = env.path("full");
    std::fs::create_dir_all(&full).unwrap();
    std::fs::write(full.join("x"), "x").unwrap();
    assert_eq!(
        env.json_error(
            &[
                "account",
                "add",
                "local",
                "--mount-point",
                full.to_str().unwrap(),
                "--param",
                &format!("root={}", env.path("home").display()),
            ],
            2
        ),
        "invalid_argument"
    );
    // A failed add leaves nothing behind.
    assert_eq!(env.json(&["account", "list"]), Value::Array(vec![]));

    env.add_local("Data");
    let all = env.json(&["config", "get"]);
    assert_eq!(all["cache-limit"], "20G");
    assert_eq!(all["quit-with-pending"], "finish-in-background");
    assert_eq!(all["Data:cache-limit"], "20G");
    assert_eq!(
        env.json(&["config", "set", "Data:cache-limit", "1G"])["Data:cache-limit"],
        "1G"
    );
    assert_eq!(
        env.json(&["config", "set", "cache-max-age", "720h"])["cache-max-age"],
        "30d"
    );
    assert_eq!(
        env.json_error(&["config", "set", "cache-limit", "lots"], 2),
        "invalid_argument"
    );
    assert_eq!(
        env.json_error(&["config", "get", "nope"], 2),
        "invalid_argument"
    );

    // Settings persist; the new cache limit applies at the next mount.
    env.json(&["config", "set", "quit-with-pending", "ask"]);
    env.stop_daemon();
    env.start_daemon();
    assert_eq!(
        env.json(&["config", "get", "quit-with-pending"])["quit-with-pending"],
        "ask"
    );
    assert_eq!(
        env.account_state("Data")["account"]["cache_limit"],
        1u64 << 30
    );
}

#[test]
fn user_folder_survives_an_rclone_crash() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let (src, mount) = env.add_local("Survivor");
    // Written through the mount: rclone does not see changes made behind
    // its back in the source folder until its directory cache expires.
    std::fs::write(mount.join("a.txt"), "a").unwrap();
    env.wait_until("a.txt is uploaded", Duration::from_secs(20), |_| {
        std::fs::read_to_string(src.join("a.txt")).is_ok_and(|s| s == "a")
    });

    let pid = std::fs::read_to_string(env.path("home/.local/share/nuage/rcd.pid")).unwrap();
    let killed = Command::new("kill")
        .args(["-9", pid.trim()])
        .status()
        .unwrap();
    assert!(killed.success());

    // The folder never disappears; reads wait for rclone to come back.
    assert_eq!(mount_type(&mount).as_deref(), Some("fuse.nuage"));
    let start = Instant::now();
    assert_eq!(std::fs::read_to_string(mount.join("a.txt")).unwrap(), "a");
    assert!(start.elapsed() < Duration::from_secs(20));
    env.wait_until(
        "the account is mounted again",
        Duration::from_secs(20),
        |env| env.account_state("Survivor")["mounted"] == true,
    );
    std::fs::write(mount.join("b.txt"), "b").unwrap();
    env.wait_until("b.txt is uploaded", Duration::from_secs(20), |_| {
        std::fs::read_to_string(src.join("b.txt")).is_ok_and(|s| s == "b")
    });
}

/// `GET` over plain HTTP; returns the status line and the `Location` header.
fn http_get(url: &str) -> (String, Option<String>) {
    let (status, location, _) = http_get_body(url);
    (status, location)
}

/// Status line, `Location` and body of a GET.
///
/// A server that was just started (rclone's sign-in page) may refuse the
/// connection or close it without an answer for a moment: retried for a
/// few seconds.
fn http_get_body(url: &str) -> (String, Option<String>, String) {
    use std::io::{Read, Write};
    let rest = url.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let attempt = || -> std::io::Result<String> {
        let mut stream = std::net::TcpStream::connect(host)?;
        write!(
            stream,
            "GET /{path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let response = loop {
        match attempt() {
            Ok(response) if !response.is_empty() => break response,
            result if Instant::now() >= deadline => {
                panic!("GET {url}: no answer ({result:?})")
            }
            _ => std::thread::sleep(Duration::from_millis(200)),
        }
    };
    let status = response.lines().next().unwrap_or_default().to_owned();
    let location = response
        .lines()
        .find_map(|l| l.strip_prefix("Location: ").map(str::to_owned));
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, location, body)
}

#[test]
fn google_sign_in_can_be_cancelled() {
    let _port = SIGN_IN_PORT.lock().unwrap_or_else(|e| e.into_inner());
    let mut env = env_or_skip!();
    env.start_daemon();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nuage"));
    env.apply_env(&mut cmd);
    let mut child = cmd
        .args([
            "account",
            "add",
            "gdrive",
            "--name",
            "Drive",
            "--client-id",
            "nuage-test.apps.googleusercontent.com",
            "--client-secret",
            "secret",
            "--no-browser",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    // The CLI prints the sign-in link and waits.
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut text = String::new();
    let mut url = None;
    while !text.contains("Waiting for the sign-in") {
        let mut line = String::new();
        let n = std::io::BufRead::read_line(&mut stderr, &mut line).unwrap();
        assert!(n > 0, "no sign-in link in:\n{text}{}", env.log());
        if let Some(start) = line.find("http://127.0.0.1:53682/") {
            url = Some(line[start..].trim().to_owned());
        }
        text.push_str(&line);
    }
    let url = url.expect("sign-in link");
    assert!(text.contains("ssh -L 53682:localhost:53682"), "{text}");

    // The link leads to Google's sign-in page with our client.
    let (status, location) = http_get(&url);
    assert!(status.contains(" 30"), "{status}");
    let location = location.unwrap();
    assert!(
        location.starts_with("https://accounts.google.com/"),
        "{location}"
    );
    assert!(location.contains("client_id=nuage-test.apps.googleusercontent.com"));

    // One sign-in at a time.
    let mut other = vec!["account", "add", "gdrive", "--name", "Other"];
    other.extend(["--client-id", "a", "--client-secret", "b", "--no-browser"]);
    assert_eq!(env.json_error(&other, 1), "busy");

    // Ctrl-C cancels: nothing is kept.
    let killed = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(4));
    assert_eq!(env.json(&["account", "list"]), Value::Array(vec![]));
    let conf =
        std::fs::read_to_string(env.path("home/.config/nuage/rclone.conf")).unwrap_or_default();
    assert!(!conf.contains("[nuage-"), "the remote is deleted:\n{conf}");
    assert!(!env.path("home/Cloud/Drive").exists());

    // The browser lands on Nuage's page, not rclone's: here a failure (the
    // answer does not match the sign-in).
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nuage"));
    env.apply_env(&mut cmd);
    let mut child = cmd
        .args([
            "account",
            "add",
            "gdrive",
            "--name",
            "Drive",
            "--client-id",
            "nuage-test.apps.googleusercontent.com",
            "--client-secret",
            "secret",
            "--no-browser",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut text = String::new();
    while !text.contains("Waiting for the sign-in") {
        let mut line = String::new();
        let n = std::io::BufRead::read_line(&mut stderr, &mut line).unwrap();
        assert!(n > 0, "no sign-in link in:\n{text}{}", env.log());
        text.push_str(&line);
    }
    let (_, _, page) = http_get_body("http://127.0.0.1:53682/?state=wrong&code=nothing");
    assert!(page.contains("The sign-in did not work"), "{page}");
    assert!(page.contains("go back to Nuage") || page.contains("Go back to Nuage"));
    assert!(!page.contains("go back to rclone"), "{page}");
    let status = child.wait().unwrap();
    assert!(!status.success());
    assert_eq!(env.json(&["account", "list"]), Value::Array(vec![]));
}

#[test]
fn exclusions() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let src = env.path("src-Ex");
    std::fs::create_dir_all(src.join("Archives")).unwrap();
    std::fs::create_dir_all(src.join("docs")).unwrap();
    std::fs::write(src.join("Archives/a.zip"), "zip").unwrap();
    std::fs::write(src.join("docs/report.odt"), "odt").unwrap();
    std::fs::write(src.join("docs/old.log"), "old").unwrap();
    let mount = env.path("home/Cloud/Ex");
    let account = env.json(&[
        "account",
        "add",
        "local",
        "--name",
        "Ex",
        "--mount-point",
        mount.to_str().unwrap(),
        "--param",
        &format!("root={}", src.display()),
    ]);
    let id = account["id"].as_str().unwrap().to_owned();
    let hidden = env.path(&format!("home/.local/share/nuage/mounts/{id}"));
    let local_only = env.path(&format!("home/.local/share/nuage/local-only/{id}"));

    // The default patterns.
    let list = env.json(&["exclude", "list", "Ex"]);
    assert_eq!(list.as_array().unwrap().len(), 4);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .all(|e| e["default"] == true)
    );

    env.json(&["exclude", "add", "Ex", "Archives"]);
    env.json(&["exclude", "add", "Ex", "--glob", "*.log"]);
    assert_eq!(
        env.json_error(&["exclude", "add", "Ex", "--glob", "*.log"], 1),
        "already_exists"
    );
    assert_eq!(
        env.json_error(&["exclude", "add", "Ex", "/"], 2),
        "invalid_argument"
    );
    // Hidden from the folder, kept in the cloud, filtered by rclone too.
    assert!(!mount.join("Archives").exists());
    assert!(!mount.join("docs/old.log").exists());
    assert!(mount.join("docs/report.odt").exists());
    assert!(src.join("Archives/a.zip").exists() && src.join("docs/old.log").exists());
    assert!(!hidden.join("Archives").exists(), "rclone filters applied");
    assert!(!hidden.join("docs/old.log").exists());

    // Excluded files created locally never leave this computer.
    std::fs::write(mount.join("docs/new.log"), "new").unwrap();
    std::fs::write(mount.join("draft.tmp"), "draft").unwrap();
    std::fs::write(mount.join("kept.txt"), "kept").unwrap();
    env.wait_until("kept.txt is uploaded", Duration::from_secs(20), |_| {
        src.join("kept.txt").exists()
    });
    std::thread::sleep(Duration::from_secs(3));
    assert!(!src.join("docs/new.log").exists());
    assert!(!src.join("draft.tmp").exists());
    assert_eq!(
        std::fs::read_to_string(local_only.join("docs/new.log")).unwrap(),
        "new"
    );
    assert_eq!(
        std::fs::read_to_string(mount.join("docs/new.log")).unwrap(),
        "new"
    );

    // Changing a rule while uploads are pending needs --yes.
    env.json(&["pause", "Ex"]);
    std::fs::write(mount.join("pending.txt"), "p").unwrap();
    env.wait_until("the write is queued", Duration::from_secs(10), |env| {
        env.account_state("Ex")["state"]["queued"] == 1
    });
    assert_eq!(
        env.json_error(&["exclude", "add", "Ex", "--glob", "*.bak"], 1),
        "busy"
    );
    env.json(&["exclude", "add", "Ex", "--glob", "*.bak", "--yes"]);
    env.json(&["resume", "Ex"]);
    // Uploaded, and no longer counted as pending: the file can reach the
    // cloud a moment before the daemon's count drops, and a rule change
    // in between is "busy".
    env.wait_until("pending.txt is uploaded", Duration::from_secs(20), |env| {
        src.join("pending.txt").exists()
            && env.account_state("Ex")["state"]["state"] == "up_to_date"
    });

    // Included again: the local file goes to the cloud, the remote one shows.
    env.json(&["exclude", "remove", "Ex", "--glob", "*.log"]);
    env.wait_until("new.log is uploaded", Duration::from_secs(20), |_| {
        std::fs::read_to_string(src.join("docs/new.log")).is_ok_and(|s| s == "new")
    });
    assert!(mount.join("docs/old.log").exists());
    assert!(!local_only.join("docs/new.log").exists());
    assert_eq!(
        env.json_error(&["exclude", "remove", "Ex", "--glob", "*.tmp"], 2),
        "invalid_argument",
        "defaults are turned off, not removed"
    );

    // Defaults off: draft.tmp goes to the cloud too.
    let set = env.json(&["config", "set", "Ex:default-exclusions", "off"]);
    assert_eq!(set["Ex:default-exclusions"], "off");
    env.wait_until("draft.tmp is uploaded", Duration::from_secs(20), |_| {
        src.join("draft.tmp").exists()
    });

    // Local-only files are protected from an account removal.
    std::fs::write(mount.join("only-here.bak"), "bak").unwrap();
    assert_eq!(
        env.json_error(&["account", "remove", "Ex", "--yes"], 1),
        "busy"
    );
    env.json(&["account", "remove", "Ex", "--yes", "--force"]);
    assert!(!local_only.exists());
    assert!(
        src.join("Archives/a.zip").exists(),
        "nothing deleted in the cloud"
    );
}

#[test]
fn moving_an_account() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let (src, mount) = env.add_local("Mover");
    std::fs::write(src.join("a.txt"), "a").unwrap();
    let new_folder = env.path("home/Elsewhere/Mover");
    let moved = env.json(&["account", "move", "Mover", new_folder.to_str().unwrap()]);
    assert_eq!(moved["mount_point"], new_folder.to_str().unwrap());
    assert!(!fuse_mounted(&mount));
    assert!(!mount.exists(), "the old empty folder is removed");
    assert!(fuse_mounted(&new_folder));
    assert_eq!(
        std::fs::read_to_string(new_folder.join("a.txt")).unwrap(),
        "a"
    );

    // A folder that is not empty is refused, and nothing changes.
    let full = env.path("home/full");
    std::fs::create_dir_all(&full).unwrap();
    std::fs::write(full.join("x"), "x").unwrap();
    assert_eq!(
        env.json_error(&["account", "move", "Mover", full.to_str().unwrap()], 2),
        "invalid_argument"
    );
    assert!(fuse_mounted(&new_folder));
}

#[test]
fn file_operations() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let src = env.path("src-Files");
    std::fs::create_dir_all(src.join("docs/sub")).unwrap();
    std::fs::write(src.join("docs/a.txt"), "aaa").unwrap();
    std::fs::write(src.join("docs/sub/b.txt"), "b").unwrap();
    std::fs::write(src.join("notes.log"), "log").unwrap();
    let (_, mount) = env.add_local("Files");
    let (other_src, _) = env.add_local("Other");
    // The mount lists the folders before the changes below: they must show
    // without waiting for its directory cache to expire.
    assert!(mount.join("docs/a.txt").exists());
    assert!(mount.join("docs/sub").is_dir());

    // ls: folders first, sizes, badges.
    let list = env.json(&["ls", "Files:"]);
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["docs", "notes.log"]);
    let list = env.json(&["ls", "Files:docs"]);
    assert_eq!(list[0]["name"], "sub");
    assert_eq!(list[0]["kind"], "folder");
    assert_eq!(list[1]["path"], "docs/a.txt");
    assert_eq!(list[1]["size"], 3);
    let out = env.nuage(&["ls", "Files:docs"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("sub/") && text.contains("a.txt"), "{text}");
    assert_eq!(env.json_error(&["ls", "Files:nope"], 1), "not_found");
    assert_eq!(env.json_error(&["ls", "Files"], 2), "usage");
    env.json(&["exclude", "add", "Files", "--glob", "*.log"]);
    let list = env.json(&["ls", "Files:"]);
    assert_eq!(list[1]["badges"][0], "not_synced", "{list}");

    // mkdir, then the mount shows it at once.
    env.json(&["mkdir", "Files:new/deep"]);
    assert!(src.join("new/deep").is_dir());
    assert!(mount.join("new/deep").is_dir());
    assert_eq!(env.json_error(&["mkdir", "Files:new"], 1), "already_exists");

    // mv and cp: renames, into folders, folders, across accounts.
    env.json(&["mv", "Files:docs/a.txt", "Files:docs/renamed.txt"]);
    assert!(!src.join("docs/a.txt").exists());
    assert_eq!(
        std::fs::read_to_string(src.join("docs/renamed.txt")).unwrap(),
        "aaa"
    );
    // Kernel caches of both FUSE layers last a second each.
    env.wait_until("the mount shows the rename", Duration::from_secs(5), |_| {
        mount.join("docs/renamed.txt").exists() && !mount.join("docs/a.txt").exists()
    });
    env.json(&["cp", "Files:docs/renamed.txt", "Files:new"]);
    assert!(src.join("new/renamed.txt").exists() && src.join("docs/renamed.txt").exists());
    env.json(&["mv", "Files:docs/sub", "Files:new"]);
    assert!(src.join("new/sub/b.txt").exists() && !src.join("docs/sub").exists());
    env.json(&["cp", "Files:new", "Other:copied"]);
    assert!(other_src.join("copied/sub/b.txt").exists());
    assert_eq!(
        env.json_error(&["mv", "Files:new", "Files:new/deep"], 2),
        "invalid_argument"
    );
    assert_eq!(
        env.json_error(
            &["cp", "Files:docs/renamed.txt", "Files:new/renamed.txt"],
            1
        ),
        "already_exists"
    );

    // upload and download, files and folders.
    let local = env.path("local");
    std::fs::create_dir_all(local.join("dir")).unwrap();
    std::fs::write(local.join("up.txt"), "up").unwrap();
    std::fs::write(local.join("dir/in.txt"), "in").unwrap();
    env.json(&[
        "upload",
        local.join("up.txt").to_str().unwrap(),
        local.join("dir").to_str().unwrap(),
        "Files:new",
    ]);
    assert_eq!(
        std::fs::read_to_string(src.join("new/up.txt")).unwrap(),
        "up"
    );
    assert_eq!(
        std::fs::read_to_string(src.join("new/dir/in.txt")).unwrap(),
        "in"
    );
    let down = env.path("down");
    std::fs::create_dir_all(&down).unwrap();
    env.json(&["download", "Files:new/sub", down.to_str().unwrap()]);
    env.json(&[
        "download",
        "Files:docs/renamed.txt",
        down.join("copy.txt").to_str().unwrap(),
    ]);
    assert_eq!(
        std::fs::read_to_string(down.join("sub/b.txt")).unwrap(),
        "b"
    );
    assert_eq!(
        std::fs::read_to_string(down.join("copy.txt")).unwrap(),
        "aaa"
    );

    // rm: no trash on `local`, so --yes; folders need --recursive.
    assert_eq!(env.json_error(&["rm", "Files:new/up.txt"], 2), "usage");
    env.json(&["rm", "Files:new/up.txt", "--yes"]);
    assert!(!src.join("new/up.txt").exists());
    env.wait_until(
        "the mount shows the deletion",
        Duration::from_secs(5),
        |_| !mount.join("new/up.txt").exists(),
    );
    assert_eq!(
        env.json_error(&["rm", "Files:new", "--yes"], 2),
        "invalid_argument"
    );
    env.json(&["rm", "Files:new/deep", "--yes"]);
    env.json(&["rm", "Files:new", "--recursive", "--yes"]);
    assert!(!src.join("new").exists());
    assert_eq!(
        env.json_error(&["rm", "Files:", "--recursive", "--yes"], 2),
        "invalid_argument"
    );

    // No share links on `local`.
    assert_eq!(
        env.json_error(&["link", "Files:docs/renamed.txt"], 1),
        "unsupported"
    );

    // Works unmounted too.
    env.json(&["unmount", "Files"]);
    env.json(&["mkdir", "Files:offline"]);
    let list = env.json(&["ls", "Files:"]);
    assert!(
        list.as_array()
            .unwrap()
            .iter()
            .any(|e| e["name"] == "offline")
    );
}

#[test]
fn cache_settings_apply_to_mounted_accounts() {
    let mut env = env_or_skip!();
    env.start_daemon();
    let (src, mount) = env.add_local("Cached");
    std::fs::write(src.join("a.txt"), "a").unwrap();

    let set = env.json(&["config", "set", "Cached:cache-max-age", "1h"]);
    assert_eq!(set["Cached:cache-max-age"], "1h");
    let set = env.json(&["config", "set", "Cached:cache-limit", "2G"]);
    assert_eq!(set["Cached:cache-limit"], "2G");
    // Applied by remounting the hidden mount: the folder stays usable.
    assert!(fuse_mounted(&mount));
    assert_eq!(std::fs::read_to_string(mount.join("a.txt")).unwrap(), "a");
    std::fs::write(mount.join("b.txt"), "b").unwrap();
    env.wait_until("b.txt is uploaded", Duration::from_secs(40), |_| {
        src.join("b.txt").exists()
    });
    let all = env.json(&["config", "get"]);
    assert_eq!(all["Cached:cache-max-age"], "1h");
    let set = env.json(&["config", "set", "Cached:cache-max-age", "default"]);
    assert_eq!(set["Cached:cache-max-age"], "default");
    assert_eq!(
        env.json_error(&["config", "set", "Cached:cache-max-age", "soon"], 2),
        "invalid_argument"
    );
    // Survives a restart.
    env.json(&["config", "set", "Cached:cache-max-age", "2w"]);
    env.stop_daemon();
    env.start_daemon();
    assert_eq!(
        env.json(&["config", "get", "Cached:cache-max-age"])["Cached:cache-max-age"],
        "2w"
    );
}

/// The account's pin store, where pinned items are kept.
fn pin_store(env: &TestEnv, name: &str) -> PathBuf {
    let list = env.json(&["account", "list"]);
    let id = list
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["display_name"] == name)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    env.path(&format!("home/.local/share/nuage/pins/{id}"))
}

fn pin_state(env: &TestEnv, path: &str) -> String {
    let pins = env.json(&["pin", "list"]);
    pins.as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"] == path)
        .map(|p| p["state"].as_str().unwrap().to_owned())
        .unwrap_or_default()
}

#[test]
fn offline_pinning() {
    let mut env = env_or_skip!();
    // While this file exists, the daemon behaves as if the network is gone.
    let offline_flag = env.path("offline");
    env.extra_env
        .push(("NUAGE_TEST_OFFLINE".into(), offline_flag.clone().into()));
    env.start_daemon();
    env.json(&["config", "set", "offline-check-interval", "1s"]);
    let src = env.path("src-Pins");
    for dir in ["Passwords", "Work/sub", "Misc"] {
        std::fs::create_dir_all(src.join(dir)).unwrap();
    }
    std::fs::write(src.join("Passwords/db.kdbx"), "v1").unwrap();
    std::fs::write(src.join("Work/a.txt"), "a").unwrap();
    std::fs::write(src.join("Work/sub/b.txt"), "b").unwrap();
    std::fs::write(src.join("Misc/c.txt"), "c").unwrap();
    std::fs::write(src.join("big.bin"), "big").unwrap();
    let (_, mount) = env.add_local("Pins");
    let store = pin_store(&env, "Pins");

    // Pinning downloads the copies before answering.
    env.json(&["pin", "add", "Pins:Passwords/db.kdbx"]);
    env.json(&["pin", "add", "Pins:Work"]);
    assert_eq!(
        std::fs::read_to_string(store.join("Passwords/db.kdbx")).unwrap(),
        "v1"
    );
    assert_eq!(
        std::fs::read_to_string(store.join("Work/sub/b.txt")).unwrap(),
        "b"
    );
    assert_eq!(pin_state(&env, "Work"), "available");
    assert_eq!(
        env.json_error(&["pin", "add", "Pins:Work/a.txt"], 1),
        "already_exists"
    );
    assert_eq!(env.json_error(&["pin", "add", "Pins:nope"], 1), "not_found");
    let text = String::from_utf8_lossy(&env.nuage(&["pin", "list"]).stdout).to_string();
    assert!(
        text.contains("Pins:Work/") && text.contains("available offline"),
        "{text}"
    );

    // Saved like KeePassXC does: a temporary file renamed over the database.
    std::fs::write(mount.join("Passwords/db.kdbx.Ab12Cd"), "v2").unwrap();
    std::fs::rename(
        mount.join("Passwords/db.kdbx.Ab12Cd"),
        mount.join("Passwords/db.kdbx"),
    )
    .unwrap();
    env.wait_until("the database is uploaded", Duration::from_secs(30), |_| {
        std::fs::read_to_string(src.join("Passwords/db.kdbx")).is_ok_and(|s| s == "v2")
    });
    assert!(!src.join("Passwords/db.kdbx.Ab12Cd").exists());

    // Remote changes and new remote files come down.
    std::fs::write(src.join("Work/a.txt"), "a, changed remotely").unwrap();
    std::fs::write(src.join("Work/new.txt"), "new").unwrap();
    env.wait_until(
        "remote changes are downloaded",
        Duration::from_secs(30),
        |_| {
            std::fs::read_to_string(store.join("Work/a.txt"))
                .is_ok_and(|s| s == "a, changed remotely")
                && store.join("Work/new.txt").exists()
        },
    );
    assert_eq!(
        std::fs::read_to_string(mount.join("Work/new.txt")).unwrap(),
        "new"
    );

    // Offline: pinned items work, others are listed and fail at once.
    std::fs::write(&offline_flag, "").unwrap();
    env.wait_until("the account is offline", Duration::from_secs(15), |env| {
        env.account_state("Pins")["state"]["state"] == "offline"
    });
    assert_eq!(
        std::fs::read_to_string(mount.join("Passwords/db.kdbx")).unwrap(),
        "v2"
    );
    std::fs::write(mount.join("Work/a.txt"), "edited offline").unwrap();
    std::fs::write(mount.join("Work/offline.txt"), "created offline").unwrap();
    let mut root: Vec<String> = std::fs::read_dir(&mount)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    root.sort();
    assert_eq!(root, ["Misc", "Passwords", "Work", "big.bin"]);
    let started = Instant::now();
    let err = std::fs::read(mount.join("big.bin")).unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc_ehostunreach()), "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
    // Changed on both sides meanwhile.
    std::fs::write(src.join("Work/sub/b.txt"), "b, remote version").unwrap();
    std::fs::write(mount.join("Work/sub/b.txt"), "b, local version").unwrap();
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        std::fs::read_to_string(src.join("Work/a.txt")).unwrap(),
        "a, changed remotely",
        "nothing is sent while offline"
    );
    assert!(env.json(&["pin", "list"])[1]["pending"].as_u64().unwrap() >= 2);

    // Back online: the changes go up, the conflict keeps both versions.
    std::fs::remove_file(&offline_flag).unwrap();
    env.wait_until(
        "offline changes are uploaded",
        Duration::from_secs(30),
        |_| {
            std::fs::read_to_string(src.join("Work/a.txt")).is_ok_and(|s| s == "edited offline")
                && src.join("Work/offline.txt").exists()
        },
    );
    env.wait_until("the conflict is resolved", Duration::from_secs(30), |_| {
        std::fs::read_dir(src.join("Work/sub")).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .contains("(conflict ")
        })
    });
    let conflict = std::fs::read_dir(src.join("Work/sub"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().contains("(conflict "))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&conflict).unwrap(),
        "b, local version"
    );
    env.wait_until(
        "the remote version is downloaded",
        Duration::from_secs(30),
        |_| {
            std::fs::read_to_string(mount.join("Work/sub/b.txt"))
                .is_ok_and(|s| s == "b, remote version")
        },
    );
    // Counted once the conflict copy is uploaded, which can be just after
    // the remote version shows in the folder.
    env.wait_until("the conflict is counted", Duration::from_secs(10), |env| {
        env.account_state("Pins")["pins_conflicts"]
            .as_u64()
            .is_some_and(|n| n >= 1)
    });

    // Restart without network: the folder is there, pinned items open.
    std::fs::write(&offline_flag, "").unwrap();
    env.stop_daemon();
    env.start_daemon();
    env.wait_mounted("Pins", &mount);
    assert_eq!(
        std::fs::read_to_string(mount.join("Passwords/db.kdbx")).unwrap(),
        "v2"
    );
    assert_eq!(
        std::fs::read_to_string(mount.join("Work/new.txt")).unwrap(),
        "new"
    );
    assert!(mount.join("Misc").is_dir(), "the tree stays browsable");
    assert_eq!(env.account_state("Pins")["state"]["state"], "offline");
    std::fs::remove_file(&offline_flag).unwrap();
    env.wait_until("back online", Duration::from_secs(20), |env| {
        env.account_state("Pins")["state"]["state"] == "up_to_date"
    });
    assert_eq!(
        std::fs::read_to_string(mount.join("Misc/c.txt")).unwrap(),
        "c"
    );

    // Unpinning waits for the changes to be uploaded.
    env.json(&["pause", "Pins"]);
    std::fs::write(mount.join("Passwords/db.kdbx"), "v3").unwrap();
    std::thread::sleep(Duration::from_secs(5));
    assert_eq!(
        env.json_error(&["pin", "remove", "Pins:Passwords/db.kdbx"], 1),
        "busy"
    );
    env.json(&["resume", "Pins"]);
    env.wait_until("v3 is uploaded", Duration::from_secs(30), |_| {
        std::fs::read_to_string(src.join("Passwords/db.kdbx")).is_ok_and(|s| s == "v3")
    });
    env.json(&["pin", "remove", "Pins:Passwords/db.kdbx"]);
    assert!(!store.join("Passwords/db.kdbx").exists());
    // Still there, now through rclone.
    assert_eq!(
        std::fs::read_to_string(mount.join("Passwords/db.kdbx")).unwrap(),
        "v3"
    );
}

fn libc_ehostunreach() -> i32 {
    113
}

#[test]
fn onedrive_sign_in_starts_without_a_client() {
    let _port = SIGN_IN_PORT.lock().unwrap_or_else(|e| e.into_inner());
    let mut env = env_or_skip!();
    env.start_daemon();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_nuage"));
    env.apply_env(&mut cmd);
    // rclone's own Microsoft client: no --client-id needed.
    let mut child = cmd
        .args(["account", "add", "onedrive", "--no-browser"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = std::io::BufReader::new(child.stderr.take().unwrap());
    let mut text = String::new();
    let mut url = None;
    while !text.contains("Waiting for the sign-in") {
        let mut line = String::new();
        let n = std::io::BufRead::read_line(&mut stderr, &mut line).unwrap();
        assert!(n > 0, "no sign-in link in:\n{text}{}", env.log());
        if let Some(start) = line.find("http://127.0.0.1:53682/") {
            url = Some(line[start..].trim().to_owned());
        }
        text.push_str(&line);
    }
    let (status, location) = http_get(&url.expect("sign-in link"));
    assert!(status.contains(" 30"), "{status}");
    let location = location.unwrap();
    assert!(
        location.starts_with("https://login.microsoftonline.com/"),
        "{location}"
    );
    let killed = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    assert_eq!(child.wait().unwrap().code(), Some(4));
    assert_eq!(env.json(&["account", "list"]), Value::Array(vec![]));
}

#[test]
fn other_drives_need_a_provider_that_has_them() {
    let mut env = env_or_skip!();
    env.start_daemon();
    env.add_local("Plain");
    assert_eq!(
        env.json_error(&["account", "drives", "Plain"], 1),
        "unsupported"
    );
    assert_eq!(
        env.json_error(
            &["account", "add", "local", "--from", "Plain", "--drive", "x"],
            1
        ),
        "unsupported"
    );
    // --from needs --drive.
    let out = env.nuage(&["account", "add", "gdrive", "--from", "Plain"]);
    assert_eq!(out.status.code(), Some(2), "{}", env.describe(&out));
}

/// An `rclone serve` process, stopped when dropped.
struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A free local port.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Serves `dir` with `rclone serve <kind>` on a free port; returns the
/// server and its address.
fn serve(env: &TestEnv, kind: &str, dir: &Path, auth: &[&str]) -> (Server, String) {
    let port = free_port();
    let address = format!("127.0.0.1:{port}");
    let child = Command::new(&env.rclone)
        .args(["serve", kind])
        .arg(dir)
        .args(["--addr", &address])
        .args(auth)
        .env("RCLONE_CONFIG", env.path("serve.conf"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let server = Server(child);
    let deadline = Instant::now() + Duration::from_secs(10);
    while std::net::TcpStream::connect(&address).is_err() {
        assert!(
            Instant::now() < deadline,
            "rclone serve {kind} did not start"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    (server, format!("http://{address}"))
}

#[test]
fn webdav_and_s3_accounts_are_checked_and_mounted() {
    let mut env = env_or_skip!();
    env.start_daemon();

    // WebDAV: a wrong password adds nothing; the right one mounts.
    let dav = env.path("dav");
    std::fs::create_dir_all(&dav).unwrap();
    std::fs::write(dav.join("a.txt"), "dav").unwrap();
    let (_dav, url) = serve(&env, "webdav", &dav, &["--user", "ana", "--pass", "pw"]);
    let add_dav = |pass: &str| {
        vec![
            "account".to_owned(),
            "add".into(),
            "webdav".into(),
            "--name".into(),
            "Dav".into(),
            "--mount-point".into(),
            env.path("home/Cloud/Dav").display().to_string(),
            "--param".into(),
            format!("url={url}"),
            "--param".into(),
            "user=ana".into(),
            "--param".into(),
            format!("pass={pass}"),
        ]
    };
    let wrong = add_dav("nope");
    let wrong: Vec<&str> = wrong.iter().map(String::as_str).collect();
    assert_eq!(env.json_error(&wrong, 2), "invalid_argument");
    assert_eq!(env.status()["accounts"], Value::Array(vec![]));
    let right = add_dav("pw");
    let right: Vec<&str> = right.iter().map(String::as_str).collect();
    env.json(&right);
    let mount = env.path("home/Cloud/Dav");
    env.wait_until("the WebDAV file", Duration::from_secs(10), |_| {
        std::fs::read_to_string(mount.join("a.txt")).is_ok_and(|s| s == "dav")
    });

    // S3: one bucket as the account root; a missing bucket adds nothing.
    let s3 = env.path("s3");
    std::fs::create_dir_all(s3.join("photos")).unwrap();
    std::fs::create_dir_all(s3.join("other")).unwrap();
    std::fs::write(s3.join("photos/p.txt"), "photo").unwrap();
    let (key, secret) = (
        "AKIAIOSFODNN7EXAMPLE",
        "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY",
    );
    let (_s3, endpoint) = serve(&env, "s3", &s3, &["--auth-key", &format!("{key},{secret}")]);
    let add_s3 = |name: &str, bucket: &str| {
        vec![
            "account".to_owned(),
            "add".into(),
            "s3".into(),
            "--name".into(),
            name.into(),
            "--mount-point".into(),
            env.path(&format!("home/Cloud/{name}"))
                .display()
                .to_string(),
            "--param".into(),
            format!("access_key_id={key}"),
            "--param".into(),
            format!("secret_access_key={secret}"),
            "--param".into(),
            format!("endpoint={endpoint}"),
            "--param".into(),
            format!("bucket={bucket}"),
        ]
    };
    let missing = add_s3("Missing", "nope");
    let missing: Vec<&str> = missing.iter().map(String::as_str).collect();
    assert_eq!(env.json_error(&missing, 2), "invalid_argument");
    let photos = add_s3("Photos", "photos");
    let photos: Vec<&str> = photos.iter().map(String::as_str).collect();
    env.json(&photos);
    let mount = env.path("home/Cloud/Photos");
    env.wait_until("the bucket's file", Duration::from_secs(10), |_| {
        std::fs::read_to_string(mount.join("p.txt")).is_ok_and(|s| s == "photo")
    });
    assert!(!mount.join("other").exists(), "only the bucket is shown");
    let listed = env.json(&["ls", "Photos:"]);
    assert!(listed.to_string().contains("p.txt"), "{listed}");
    // Writing goes into the bucket.
    std::fs::write(mount.join("new.txt"), "new").unwrap();
    env.wait_until("the upload", Duration::from_secs(30), |_| {
        s3.join("photos/new.txt").exists()
    });
}
