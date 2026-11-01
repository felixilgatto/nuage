use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use nuage_core::Error;

use crate::RcClient;

/// How long rcd may take to answer `core/version` after being spawned.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long rcd may take to exit after `core/quit`.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// How to launch `rclone rcd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcdConfig {
    /// The bundled rclone binary.
    pub rclone_path: PathBuf,
    /// `RCLONE_CONFIG`.
    pub config_path: PathBuf,
    /// VFS cache directory (`--cache-dir`).
    pub cache_dir: PathBuf,
    /// Where the PID of the running rcd is written, so that an rcd left
    /// behind by a killed daemon can be stopped at the next start.
    pub pid_file: Option<PathBuf>,
}

/// Launches and watches `rclone rcd` as a child process of the daemon.
///
/// rcd listens on 127.0.0.1 with a random port and random credentials, never
/// with `--rc-no-auth`. The credentials go through the environment, not the
/// command line, so other users cannot read them with `ps`.
pub struct RcdSupervisor {
    config: RcdConfig,
    /// `RCLONE_CONFIG_PASS`, when `rclone.conf` is encrypted.
    config_password: Option<String>,
    child: Option<Child>,
    client: Option<RcClient>,
    version: Option<String>,
}

impl std::fmt::Debug for RcdSupervisor {
    // The configuration password must never reach a log.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RcdSupervisor")
            .field("config", &self.config)
            .field(
                "config_password",
                &self.config_password.as_ref().map(|_| "<redacted>"),
            )
            .field("running", &self.child.is_some())
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

impl RcdSupervisor {
    pub fn new(config: RcdConfig) -> Self {
        Self {
            config,
            config_password: None,
            child: None,
            client: None,
            version: None,
        }
    }

    pub fn config(&self) -> &RcdConfig {
        &self.config
    }

    /// Password of an encrypted `rclone.conf`, used from the next start.
    pub fn set_config_password(&mut self, password: Option<String>) {
        self.config_password = password;
    }

    /// Starts rcd and waits until `core/version` answers.
    ///
    /// An rcd recorded in the PID file (left by a daemon killed with
    /// `SIGKILL`) is stopped first.
    pub async fn start(&mut self) -> Result<RcClient, Error> {
        if self.child.is_some() {
            self.stop().await?;
        }
        if let Some(pid_file) = &self.config.pid_file {
            kill_stale_rcd(pid_file).await;
        }
        std::fs::create_dir_all(&self.config.cache_dir)?;
        if let Some(dir) = self.config.config_path.parent() {
            std::fs::create_dir_all(dir)?;
        }

        // The port is chosen by binding port 0, then released for rclone.
        // Another process could grab it in between: retry a few times.
        let mut last_err = None;
        for _ in 0..3 {
            match self.spawn_once().await {
                Ok(client) => return Ok(client),
                Err(e) => {
                    tracing::warn!(error = %e, "rclone rcd did not start, retrying");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Rclone("rclone rcd did not start".to_owned())))
    }

    async fn spawn_once(&mut self) -> Result<RcClient, Error> {
        let port = free_port()?;
        let user = random_hex(16)?;
        let pass = random_hex(32)?;
        let addr = format!("127.0.0.1:{port}");

        let mut command = Command::new(&self.config.rclone_path);
        command
            .arg("rcd")
            .arg("--rc-addr")
            .arg(&addr)
            .arg("--cache-dir")
            .arg(&self.config.cache_dir)
            .env("RCLONE_CONFIG", &self.config.config_path)
            .env("RCLONE_RC_USER", &user)
            .env("RCLONE_RC_PASS", &pass)
            // Never let rclone open a browser or read a terminal by itself.
            .env_remove("RCLONE_RC_NO_AUTH")
            .env_remove("RCLONE_PASSWORD_COMMAND")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match &self.config_password {
            Some(password) => command.env("RCLONE_CONFIG_PASS", password),
            None => command.env_remove("RCLONE_CONFIG_PASS"),
        };
        let mut child = command.spawn().map_err(|e| {
            Error::Rclone(format!(
                "cannot run {}: {e}",
                self.config.rclone_path.display()
            ))
        })?;
        forward_output(&mut child);

        let client = RcClient::new(&format!("http://{addr}/"), &user, &pass)
            .map_err(nuage_core::Error::from)?;
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(Error::Rclone(format!(
                    "rclone rcd exited during startup ({status})"
                )));
            }
            match client.core_version().await {
                Ok(v) => {
                    tracing::info!(version = %v.version, pid = child.id(), "rclone rcd started");
                    self.version = Some(v.version);
                    break;
                }
                Err(_) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => {
                    let _ = child.kill().await;
                    return Err(Error::Rclone(format!(
                        "rclone rcd does not answer on {addr}: {e}"
                    )));
                }
            }
        }

        if let (Some(pid_file), Some(pid)) = (&self.config.pid_file, child.id()) {
            if let Some(dir) = pid_file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(pid_file, format!("{pid}\n"))?;
        }
        self.child = Some(child);
        self.client = Some(client.clone());
        Ok(client)
    }

    /// Client of the running rcd, if started.
    pub fn client(&self) -> Option<&RcClient> {
        self.client.as_ref()
    }

    /// Version of the running rclone, e.g. `"v1.75.2"`.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Returns the exit status if rcd died since it was started.
    pub fn exited(&mut self) -> Option<ExitStatus> {
        let status = self.child.as_mut()?.try_wait().ok()??;
        self.child = None;
        self.client = None;
        Some(status)
    }

    /// Stops rcd with `core/quit`, after the engine has unmounted everything.
    /// Kills it if it does not exit in time.
    pub async fn stop(&mut self) -> Result<(), Error> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        if let Some(client) = self.client.take()
            && let Err(e) = client.core_quit().await
        {
            tracing::debug!(error = %e, "core/quit failed");
        }
        match tokio::time::timeout(STOP_TIMEOUT, child.wait()).await {
            Ok(status) => tracing::info!(status = %status?, "rclone rcd stopped"),
            Err(_) => {
                tracing::warn!("rclone rcd did not stop in time, killing it");
                child.kill().await?;
            }
        }
        if let Some(pid_file) = &self.config.pid_file {
            let _ = std::fs::remove_file(pid_file);
        }
        Ok(())
    }
}

/// Remotes whose cache was deleted while rclone still used it.
static REMOVED_REMOTES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// `remote` was deleted with its cache. rclone keeps its VFS after the
/// unmount: files other programs still had open are closed later, and
/// rclone then fails to write their state into the deleted cache. Those
/// expected errors go to the debug log.
pub fn cache_deleted(remote: &str) {
    if let Ok(mut removed) = REMOVED_REMOTES.lock() {
        removed.push(remote.to_owned());
    }
}

/// Whether `line` is about the cache of a removed remote.
fn about_deleted_cache(line: &str) -> bool {
    REMOVED_REMOTES.lock().is_ok_and(|removed| {
        removed.iter().any(|r| {
            line.contains(&format!("/vfsMeta/{r}/")) || line.contains(&format!("/vfs/{r}/"))
        })
    })
}

fn log_line(line: &str) {
    if about_deleted_cache(line) {
        tracing::debug!(target: "rclone", "{line}");
    } else {
        tracing::info!(target: "rclone", "{line}");
    }
}

/// Sends rclone's output to the daemon log.
fn forward_output(child: &mut Child) {
    if let Some(out) = child.stdout.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log_line(&line);
            }
        });
    }
    if let Some(err) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log_line(&line);
            }
        });
    }
}

/// Stops the rcd recorded in `pid_file`, if it is still running.
async fn kill_stale_rcd(pid_file: &Path) {
    let Ok(text) = std::fs::read_to_string(pid_file) else {
        return;
    };
    let _ = std::fs::remove_file(pid_file);
    let Some(pid) = text
        .trim()
        .parse::<i32>()
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    // Only kill it if it still is an rclone rcd: the PID may have been reused.
    let cmdline =
        std::fs::read(format!("/proc/{}/cmdline", pid.as_raw_nonzero())).unwrap_or_default();
    let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
    let is_rcd =
        args.first().is_some_and(|a| a.ends_with(b"rclone")) && args.contains(&&b"rcd"[..]);
    if !is_rcd {
        return;
    }
    tracing::info!(
        pid = pid.as_raw_nonzero(),
        "stopping rclone rcd left by a previous daemon"
    );
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::TERM);
    for _ in 0..50 {
        if !Path::new(&format!("/proc/{}", pid.as_raw_nonzero())).exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
}

fn free_port() -> Result<u16, Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// `bytes` random bytes from the kernel, hex-encoded.
fn random_hex(bytes: usize) -> Result<String, Error> {
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {

    #[test]
    fn deleted_cache_lines() {
        let line = "ERROR : a.txt: vfs cache: item close failed: open /c/rclone/vfsMeta/nuage-dead/a.txt: no such file";
        assert!(!about_deleted_cache(line));
        cache_deleted("nuage-dead");
        assert!(about_deleted_cache(line));
        assert!(!about_deleted_cache(
            "open /c/rclone/vfsMeta/nuage-alive/a.txt"
        ));
    }

    use super::*;

    #[test]
    fn debug_hides_the_password() {
        let mut rcd = RcdSupervisor::new(RcdConfig {
            rclone_path: "rclone".into(),
            config_path: "rclone.conf".into(),
            cache_dir: "cache".into(),
            pid_file: None,
        });
        rcd.set_config_password(Some("hunter2".into()));
        let shown = format!("{rcd:?}");
        assert!(!shown.contains("hunter2"), "{shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn random_hex_is_random() {
        let a = random_hex(16).unwrap();
        let b = random_hex(16).unwrap();
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn missing_binary_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut rcd = RcdSupervisor::new(RcdConfig {
            rclone_path: dir.path().join("no-rclone"),
            config_path: dir.path().join("rclone.conf"),
            cache_dir: dir.path().join("cache"),
            pid_file: None,
        });
        let err = rcd.start().await.unwrap_err();
        assert!(matches!(err, Error::Rclone(_)), "{err:?}");
    }
}
