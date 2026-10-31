use std::path::PathBuf;

use nuage_core::Error;

use crate::RcClient;

/// How to launch `rclone rcd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RcdConfig {
    /// The bundled rclone binary.
    pub rclone_path: PathBuf,
    /// `RCLONE_CONFIG`.
    pub config_path: PathBuf,
    /// VFS cache directory (`--cache-dir`).
    pub cache_dir: PathBuf,
}

/// Launches and watches `rclone rcd` as a child process of the daemon.
///
/// rcd listens on 127.0.0.1 with a random port and random credentials. The
/// supervisor restarts it if it dies and stops it with `core/quit`.
///
/// **Stub**: implemented in milestone 3.
#[derive(Debug)]
pub struct RcdSupervisor {
    config: RcdConfig,
    client: Option<RcClient>,
}

impl RcdSupervisor {
    pub fn new(config: RcdConfig) -> Self {
        Self {
            config,
            client: None,
        }
    }

    pub fn config(&self) -> &RcdConfig {
        &self.config
    }

    /// Starts rcd and waits until `core/version` answers.
    pub async fn start(&mut self) -> Result<RcClient, Error> {
        Err(Error::not_implemented("starting rclone rcd"))
    }

    /// Client of the running rcd, if started.
    pub fn client(&self) -> Option<&RcClient> {
        self.client.as_ref()
    }

    /// Stops rcd with `core/quit`, after the engine has unmounted everything.
    pub async fn stop(&mut self) -> Result<(), Error> {
        Err(Error::not_implemented("stopping rclone rcd"))
    }
}
