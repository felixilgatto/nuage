//! Nuage IPC: the protocol spoken between `nuaged` and its clients.
//!
//! - Transport: a Unix socket (mode `0600`), see [`socket_path`].
//! - Framing: newline-delimited JSON, one [`ClientMessage`] or
//!   [`ServerMessage`] per line.
//! - Session: the client sends [`ClientMessage::Hello`] with its
//!   [`PROTOCOL_VERSION`]; the daemon answers [`ServerMessage::Welcome`] or
//!   [`ServerMessage::Rejected`] (then closes the connection). The client then
//!   sends [`ClientMessage::Request`]s, each answered by a
//!   [`ServerMessage::Response`] with the same `id`, and may
//!   [`ClientMessage::Subscribe`] to the stream of [`nuage_core::Event`]s.

mod client;
mod error;
mod path;
mod protocol;
mod server;

pub use client::{Client, Subscription};
pub use error::IpcError;
pub use path::{SocketEnv, candidate_socket_paths, socket_path};
pub use protocol::{ClientMessage, Outcome, PROTOCOL_VERSION, ServerMessage};
pub use server::{Handler, Server};
