use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use nuage_core::{Command, Error, Event, Reply};

use crate::protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage};
use crate::{IpcError, protocol};

/// A connection to the daemon, after a successful handshake.
#[derive(Debug)]
pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: u64,
    daemon_version: String,
    path: PathBuf,
}

impl Client {
    /// Connects to the daemon listening on `path` and performs the handshake.
    ///
    /// `client_name` appears in the daemon logs (e.g. `"nuage 0.1.0"`).
    pub async fn connect(path: &Path, client_name: &str) -> Result<Self, IpcError> {
        let stream = UnixStream::connect(path)
            .await
            .map_err(|source| IpcError::Unreachable {
                path: path.to_owned(),
                source,
            })?;
        let (read, write) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(read),
            writer: write,
            next_id: 1,
            daemon_version: String::new(),
            path: path.to_owned(),
        };
        client
            .send(&ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client: client_name.to_owned(),
            })
            .await?;
        match client.recv().await? {
            ServerMessage::Welcome {
                protocol_version,
                daemon_version,
            } if protocol_version == PROTOCOL_VERSION => {
                client.daemon_version = daemon_version;
                Ok(client)
            }
            ServerMessage::Welcome {
                protocol_version, ..
            } => Err(IpcError::ProtocolMismatch(protocol::mismatch_message(
                PROTOCOL_VERSION,
                protocol_version,
            ))),
            ServerMessage::Rejected { message, .. } => Err(IpcError::ProtocolMismatch(message)),
            _ => Err(IpcError::UnexpectedMessage("daemon during the handshake")),
        }
    }

    /// Tries each path in order and returns the first successful connection.
    /// When no daemon is reachable, returns the error for the first path.
    pub async fn connect_any(paths: &[PathBuf], client_name: &str) -> Result<Self, IpcError> {
        let mut first_err = None;
        for path in paths {
            match Self::connect(path, client_name).await {
                Ok(client) => return Ok(client),
                Err(e @ IpcError::Unreachable { .. }) => {
                    first_err.get_or_insert(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(first_err.unwrap_or(IpcError::NoRuntimeDir))
    }

    /// Version of the daemon, as announced during the handshake.
    pub fn daemon_version(&self) -> &str {
        &self.daemon_version
    }

    /// Socket this client is connected to.
    pub fn socket_path(&self) -> &Path {
        &self.path
    }

    /// Sends a command and waits for its reply.
    ///
    /// The outer `Result` is the transport; the inner one is the outcome of
    /// the command as decided by the daemon.
    pub async fn request(&mut self, command: Command) -> Result<Result<Reply, Error>, IpcError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&ClientMessage::Request { id, command }).await?;
        // This client is sequential: the next response is ours. Anything
        // else is a protocol violation.
        match self.recv().await? {
            ServerMessage::Response {
                id: reply_id,
                outcome,
            } if reply_id == id => Ok(outcome.into()),
            _ => Err(IpcError::UnexpectedMessage("daemon")),
        }
    }

    /// Turns this connection into a stream of events.
    pub async fn subscribe(mut self) -> Result<Subscription, IpcError> {
        self.send(&ClientMessage::Subscribe).await?;
        match self.recv().await? {
            ServerMessage::Subscribed => Ok(Subscription { client: self }),
            _ => Err(IpcError::UnexpectedMessage("daemon while subscribing")),
        }
    }

    async fn send(&mut self, msg: &ClientMessage) -> Result<(), IpcError> {
        let mut line = serde_json::to_vec(msg)?;
        line.push(b'\n');
        self.writer.write_all(&line).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<ServerMessage, IpcError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line).await? == 0 {
            return Err(IpcError::ConnectionClosed);
        }
        Ok(serde_json::from_str(&line)?)
    }
}

/// Stream of events pushed by the daemon.
#[derive(Debug)]
pub struct Subscription {
    client: Client,
}

impl Subscription {
    /// Waits for the next event. Returns `Ok(None)` when the daemon closes
    /// the connection.
    pub async fn next_event(&mut self) -> Result<Option<Event>, IpcError> {
        match self.client.recv().await {
            Ok(ServerMessage::Event { event }) => Ok(Some(event)),
            Ok(_) => Err(IpcError::UnexpectedMessage("daemon in the event stream")),
            Err(IpcError::ConnectionClosed) => Ok(None),
            Err(e) => Err(e),
        }
    }
}
