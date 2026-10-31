//! Location of the daemon socket.
//!
//! - Native: `$XDG_RUNTIME_DIR/nuage/nuaged.sock`.
//! - Inside the Flatpak: `$XDG_RUNTIME_DIR/app/io.github.felixilgatto.Nuage/nuaged.sock`.
//!   That directory is shared with the host, so a CLI installed on the host
//!   can reach a daemon running in the sandbox.
//! - `NUAGE_SOCKET` overrides both (tests, development).

use std::path::{Path, PathBuf};

use nuage_core::APP_ID;

use crate::IpcError;

const SOCKET_NAME: &str = "nuaged.sock";

/// The parts of the environment that decide where the socket lives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SocketEnv {
    /// `NUAGE_SOCKET`.
    pub override_path: Option<PathBuf>,
    /// `XDG_RUNTIME_DIR`.
    pub runtime_dir: Option<PathBuf>,
    /// Whether this process runs inside the Nuage Flatpak sandbox.
    pub in_flatpak: bool,
}

impl SocketEnv {
    /// Reads the current process environment.
    pub fn from_process() -> Self {
        let non_empty = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Self {
            override_path: non_empty("NUAGE_SOCKET"),
            runtime_dir: non_empty("XDG_RUNTIME_DIR"),
            in_flatpak: std::env::var_os("FLATPAK_ID").is_some()
                || Path::new("/.flatpak-info").exists(),
        }
    }

    fn native(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("nuage").join(SOCKET_NAME)
    }

    fn flatpak(runtime_dir: &Path) -> PathBuf {
        runtime_dir.join("app").join(APP_ID).join(SOCKET_NAME)
    }

    /// Path where a daemon started in this environment listens.
    pub fn listen_path(&self) -> Result<PathBuf, IpcError> {
        if let Some(p) = &self.override_path {
            return Ok(p.clone());
        }
        let dir = self.runtime_dir.as_deref().ok_or(IpcError::NoRuntimeDir)?;
        Ok(if self.in_flatpak {
            Self::flatpak(dir)
        } else {
            Self::native(dir)
        })
    }

    /// Paths a client tries, in order: the daemon of its own environment
    /// first, then (on the host) a daemon running inside the Flatpak.
    pub fn candidates(&self) -> Result<Vec<PathBuf>, IpcError> {
        if let Some(p) = &self.override_path {
            return Ok(vec![p.clone()]);
        }
        let dir = self.runtime_dir.as_deref().ok_or(IpcError::NoRuntimeDir)?;
        Ok(if self.in_flatpak {
            vec![Self::flatpak(dir)]
        } else {
            vec![Self::native(dir), Self::flatpak(dir)]
        })
    }
}

/// Socket path the daemon listens on, from the process environment.
pub fn socket_path() -> Result<PathBuf, IpcError> {
    SocketEnv::from_process().listen_path()
}

/// Socket paths a client should try, from the process environment.
pub fn candidate_socket_paths() -> Result<Vec<PathBuf>, IpcError> {
    SocketEnv::from_process().candidates()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(flatpak: bool) -> SocketEnv {
        SocketEnv {
            override_path: None,
            runtime_dir: Some("/run/user/1000".into()),
            in_flatpak: flatpak,
        }
    }

    #[test]
    fn native_path() {
        assert_eq!(
            env(false).listen_path().unwrap(),
            Path::new("/run/user/1000/nuage/nuaged.sock")
        );
        assert_eq!(
            env(false).candidates().unwrap(),
            vec![
                PathBuf::from("/run/user/1000/nuage/nuaged.sock"),
                PathBuf::from("/run/user/1000/app/io.github.felixilgatto.Nuage/nuaged.sock"),
            ]
        );
    }

    #[test]
    fn flatpak_path() {
        assert_eq!(
            env(true).listen_path().unwrap(),
            Path::new("/run/user/1000/app/io.github.felixilgatto.Nuage/nuaged.sock")
        );
    }

    #[test]
    fn override_and_missing_runtime_dir() {
        let e = SocketEnv {
            override_path: Some("/tmp/x.sock".into()),
            ..SocketEnv::default()
        };
        assert_eq!(e.listen_path().unwrap(), Path::new("/tmp/x.sock"));
        assert!(matches!(
            SocketEnv::default().listen_path(),
            Err(IpcError::NoRuntimeDir)
        ));
    }
}
