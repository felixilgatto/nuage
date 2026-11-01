use std::io;
use std::path::PathBuf;

/// Errors of the IPC layer (transport and protocol), as opposed to errors
/// returned by the daemon for a command ([`nuage_core::Error`]).
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// No daemon is listening on the socket.
    #[error(
        "Cannot reach the Nuage daemon at {path}. Start it with: systemctl --user start nuaged"
    )]
    Unreachable {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// Another daemon already listens on the socket.
    #[error("Nuage daemon is already running (socket {0})")]
    AlreadyRunning(PathBuf),
    /// Unix socket paths are limited to 107 bytes by the kernel.
    #[error("The socket path {0} is too long (at most 107 bytes); choose a shorter NUAGE_SOCKET")]
    PathTooLong(PathBuf),
    /// `$XDG_RUNTIME_DIR` is not set, so the socket location is unknown.
    #[error(
        "XDG_RUNTIME_DIR is not set; cannot locate the daemon socket (set NUAGE_SOCKET to override)"
    )]
    NoRuntimeDir,
    /// Client and daemon speak different protocol versions.
    #[error("{0}")]
    ProtocolMismatch(String),
    #[error("The daemon closed the connection")]
    ConnectionClosed,
    #[error("Unexpected message from the {0}")]
    UnexpectedMessage(&'static str),
    #[error("Malformed message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error on the daemon socket: {0}")]
    Io(#[from] io::Error),
}
