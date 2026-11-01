use serde::{Deserialize, Serialize};

use nuage_core::{Command, Error, Event, Reply};

/// Version of the IPC protocol. Bumped on every incompatible change; the
/// daemon rejects clients speaking another version.
pub const PROTOCOL_VERSION: u32 = 2;

/// A line sent by a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// First message of every connection.
    Hello {
        protocol_version: u32,
        /// Free-form client name, for logs (e.g. `"nuage 0.1.0"`).
        client: String,
    },
    Request {
        id: u64,
        command: Command,
    },
    /// Start receiving [`ServerMessage::Event`]s on this connection.
    Subscribe,
}

/// A line sent by the daemon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        protocol_version: u32,
        daemon_version: String,
    },
    /// The handshake failed; the daemon closes the connection.
    Rejected {
        protocol_version: u32,
        message: String,
    },
    Response {
        id: u64,
        #[serde(flatten)]
        outcome: Outcome,
    },
    /// Acknowledges [`ClientMessage::Subscribe`]: every event published from
    /// now on is sent on this connection.
    Subscribed,
    Event {
        event: Event,
    },
}

/// Result of a request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Reply),
    Error(Error),
}

impl From<Result<Reply, Error>> for Outcome {
    fn from(r: Result<Reply, Error>) -> Self {
        match r {
            Ok(reply) => Outcome::Ok(reply),
            Err(e) => Outcome::Error(e),
        }
    }
}

impl From<Outcome> for Result<Reply, Error> {
    fn from(o: Outcome) -> Self {
        match o {
            Outcome::Ok(reply) => Ok(reply),
            Outcome::Error(e) => Err(e),
        }
    }
}

/// Explains a protocol version mismatch to the user.
pub(crate) fn mismatch_message(client: u32, daemon: u32) -> String {
    let advice = if client < daemon {
        "this client is older than the running daemon; update it"
    } else {
        "the running daemon is older than this client; restart it after updating \
         (systemctl --user restart nuaged)"
    };
    format!(
        "Incompatible Nuage versions: client speaks protocol v{client}, daemon speaks v{daemon} ({advice})."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::StatusReport;

    #[test]
    fn wire_format() {
        let hello = ClientMessage::Hello {
            protocol_version: 1,
            client: "test".into(),
        };
        assert_eq!(
            serde_json::to_string(&hello).unwrap(),
            r#"{"type":"hello","protocol_version":1,"client":"test"}"#
        );
        let req = ClientMessage::Request {
            id: 7,
            command: Command::Status,
        };
        assert_eq!(
            serde_json::to_string(&req).unwrap(),
            r#"{"type":"request","id":7,"command":{"command":"status"}}"#
        );

        let resp = ServerMessage::Response {
            id: 7,
            outcome: Outcome::Ok(Reply::Status(StatusReport::empty())),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.starts_with(r#"{"type":"response","id":7,"ok":{"type":"status""#));
        assert_eq!(serde_json::from_str::<ServerMessage>(&json).unwrap(), resp);

        let err = ServerMessage::Response {
            id: 8,
            outcome: Outcome::Error(Error::not_implemented("mount")),
        };
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(
            json,
            r#"{"type":"response","id":8,"error":{"code":"not_implemented","message":"mount"}}"#
        );
        assert_eq!(serde_json::from_str::<ServerMessage>(&json).unwrap(), err);
    }
}
