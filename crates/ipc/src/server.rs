use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};

use nuage_core::{Command, Error, Event, Reply};

use crate::IpcError;
use crate::protocol::{self, ClientMessage, Outcome, PROTOCOL_VERSION, ServerMessage};

/// Executes commands on behalf of the server. Implemented by the daemon.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, command: Command) -> impl Future<Output = Result<Reply, Error>> + Send;
}

/// The daemon side of the socket.
///
/// The socket file is removed when the server is dropped.
#[derive(Debug)]
pub struct Server {
    listener: UnixListener,
    path: PathBuf,
}

impl Server {
    /// Creates the socket at `path` (mode `0600`, parent directory `0700`).
    ///
    /// Fails with [`IpcError::AlreadyRunning`] if a daemon already answers
    /// on that socket. A stale socket left by a crashed daemon is replaced.
    pub async fn bind(path: &Path) -> Result<Self, IpcError> {
        check_path_len(path)?;
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
        match UnixStream::connect(path).await {
            Ok(_) => return Err(IpcError::AlreadyRunning(path.to_owned())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {
                tracing::info!(path = %path.display(), "removing stale socket");
                match std::fs::remove_file(path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            listener,
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Accepts connections forever. Each connection is served in its own
    /// task; events sent on `events` reach every subscribed client.
    ///
    /// Drop the returned future (e.g. in a `select!` with a shutdown signal)
    /// to stop the server.
    pub async fn serve<H: Handler>(
        self,
        handler: Arc<H>,
        events: broadcast::Sender<Event>,
    ) -> Result<(), IpcError> {
        loop {
            let (stream, _) = self.listener.accept().await?;
            let handler = Arc::clone(&handler);
            let events = events.clone();
            tokio::spawn(async move {
                if let Err(e) = serve_connection(stream, handler, events).await {
                    tracing::debug!(error = %e, "client connection ended with an error");
                }
            });
        }
    }
}

/// `sun_path` holds 108 bytes including the terminating NUL.
pub(crate) fn check_path_len(path: &Path) -> Result<(), IpcError> {
    use std::os::unix::ffi::OsStrExt;
    if path.as_os_str().as_bytes().len() > 107 {
        return Err(IpcError::PathTooLong(path.to_owned()));
    }
    Ok(())
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn serve_connection<H: Handler>(
    stream: UnixStream,
    handler: Arc<H>,
    events: broadcast::Sender<Event>,
) -> Result<(), IpcError> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();

    // All writes go through one task so that responses and events never
    // interleave within a line.
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(64);
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let mut line = serde_json::to_vec(&msg)?;
            line.push(b'\n');
            write.write_all(&line).await?;
        }
        Ok::<_, IpcError>(())
    });

    // Handshake.
    let Some(first) = lines.next_line().await? else {
        return Ok(());
    };
    let rejection = match serde_json::from_str::<ClientMessage>(&first) {
        Ok(ClientMessage::Hello {
            protocol_version,
            client,
        }) if protocol_version == PROTOCOL_VERSION => {
            tracing::debug!(%client, "client connected");
            None
        }
        Ok(ClientMessage::Hello {
            protocol_version, ..
        }) => Some(protocol::mismatch_message(
            protocol_version,
            PROTOCOL_VERSION,
        )),
        _ => Some("Expected a hello message first.".to_owned()),
    };
    if let Some(message) = rejection {
        let _ = tx
            .send(ServerMessage::Rejected {
                protocol_version: PROTOCOL_VERSION,
                message,
            })
            .await;
        drop(tx);
        return writer.await.map_err(|_| IpcError::ConnectionClosed)?;
    }
    let _ = tx
        .send(ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            daemon_version: nuage_core::VERSION.to_owned(),
        })
        .await;

    let mut forwarder: Option<tokio::task::JoinHandle<()>> = None;
    while let Some(line) = lines.next_line().await? {
        match serde_json::from_str::<ClientMessage>(&line) {
            Ok(ClientMessage::Request { id, command }) => {
                let handler = Arc::clone(&handler);
                let tx = tx.clone();
                tokio::spawn(async move {
                    let outcome = Outcome::from(handler.handle(command).await);
                    let _ = tx.send(ServerMessage::Response { id, outcome }).await;
                });
            }
            Ok(ClientMessage::Subscribe) if forwarder.is_none() => {
                let mut rx = events.subscribe();
                let tx = tx.clone();
                let _ = tx.send(ServerMessage::Subscribed).await;
                forwarder = Some(tokio::spawn(async move {
                    loop {
                        match rx.recv().await {
                            Ok(event) => {
                                if tx.send(ServerMessage::Event { event }).await.is_err() {
                                    break;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                tracing::warn!(skipped = n, "slow subscriber, events dropped");
                            }
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }));
            }
            Ok(ClientMessage::Subscribe) => {
                let _ = tx.send(ServerMessage::Subscribed).await;
            }
            Ok(ClientMessage::Hello { .. }) => {
                tracing::debug!("ignoring repeated hello");
            }
            Err(e) => {
                tracing::warn!(error = %e, "ignoring malformed client message");
                // A request from another version of Nuage, which this
                // daemon does not know: answered, so that the client does
                // not wait forever.
                if let Some(id) = protocol::request_id(&line) {
                    let outcome = Outcome::Error(Error::Unsupported(
                        "The running Nuage service is from another version and cannot do this. \
                         Quit Nuage and open it again."
                            .to_owned(),
                    ));
                    let _ = tx.send(ServerMessage::Response { id, outcome }).await;
                }
            }
        }
    }
    if let Some(f) = forwarder {
        f.abort();
    }
    Ok(())
}
