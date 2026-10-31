//! Client/server round trip over a real Unix socket.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

use nuage_core::{AccountId, Command, Error, Event, Reply, StatusReport};
use nuage_ipc::{Client, Handler, IpcError, PROTOCOL_VERSION, Server, ServerMessage};

struct TestHandler;

impl Handler for TestHandler {
    async fn handle(&self, command: Command) -> Result<Reply, Error> {
        match command {
            Command::Status => Ok(Reply::Status(StatusReport::empty())),
            other => Err(Error::not_implemented(format!("{other:?}"))),
        }
    }
}

async fn start() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    broadcast::Sender<Event>,
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run/nuaged.sock");
    let server = Server::bind(&path).await.unwrap();
    let (events, _) = broadcast::channel(16);
    tokio::spawn(server.serve(Arc::new(TestHandler), events.clone()));
    (dir, path, events)
}

#[tokio::test]
async fn request_response_and_events() {
    let (_dir, path, events) = start().await;

    let mut client = Client::connect(&path, "test").await.unwrap();
    assert_eq!(client.daemon_version(), nuage_core::VERSION);

    let reply = client.request(Command::Status).await.unwrap();
    assert_eq!(reply, Ok(Reply::Status(StatusReport::empty())));

    let reply = client
        .request(Command::Mount {
            account: AccountId::new("Drive"),
        })
        .await
        .unwrap();
    assert!(matches!(reply, Err(Error::NotImplemented(_))));

    let mut sub = client.subscribe().await.unwrap();
    events.send(Event::DaemonStopping).unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), sub.next_event())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event, Some(Event::DaemonStopping));
}

#[tokio::test]
async fn socket_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path, _events) = start().await;
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[tokio::test]
async fn protocol_mismatch_is_rejected() {
    let (_dir, path, _events) = start().await;
    let stream = UnixStream::connect(&path).await.unwrap();
    let (read, mut write) = stream.into_split();
    write
        .write_all(b"{\"type\":\"hello\",\"protocol_version\":999,\"client\":\"future\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await.unwrap();
    match serde_json::from_str::<ServerMessage>(&line).unwrap() {
        ServerMessage::Rejected {
            protocol_version,
            message,
        } => {
            assert_eq!(protocol_version, PROTOCOL_VERSION);
            assert!(message.contains("v999"), "{message}");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn second_daemon_is_refused_and_stale_socket_replaced() {
    let (_dir, path, _events) = start().await;
    assert!(matches!(
        Server::bind(&path).await,
        Err(IpcError::AlreadyRunning(_))
    ));

    // A socket file nobody listens on is stale and gets replaced.
    let dir = tempfile::tempdir().unwrap();
    let stale = dir.path().join("nuaged.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    assert!(stale.exists());
    let server = Server::bind(&stale).await.unwrap();
    drop(server);
    assert!(!stale.exists());
}

#[tokio::test]
async fn unreachable_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let err = Client::connect(&dir.path().join("none.sock"), "test")
        .await
        .unwrap_err();
    assert!(matches!(err, IpcError::Unreachable { .. }));
}
