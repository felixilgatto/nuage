//! `nuaged`: the Nuage daemon.
//!
//! Owns all the state (accounts, mounts, exclusions, pins), drives rclone
//! through the engine, and serves the GTK app, the tray and the CLI over a
//! Unix socket. One daemon per user: a second instance detects the first
//! through the socket and exits.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::broadcast;

use nuage_core::{Command, DaemonInfo, Error, Event, Reply};
use nuage_engine::{Engine, ProviderRegistry};
use nuage_ipc::{Handler, IpcError, PROTOCOL_VERSION, Server};

/// Nuage daemon: mounts cloud accounts with rclone and serves Nuage clients.
#[derive(Debug, Parser)]
#[command(name = "nuaged", version)]
struct Args {
    /// Socket to listen on (default: $XDG_RUNTIME_DIR/nuage/nuaged.sock).
    #[arg(long, env = "NUAGE_SOCKET", value_name = "PATH")]
    socket: Option<PathBuf>,
}

struct Daemon {
    engine: Engine,
}

impl Handler for Daemon {
    async fn handle(&self, command: Command) -> Result<Reply, Error> {
        tracing::debug!(?command, "command");
        match command {
            Command::DaemonInfo => Ok(Reply::DaemonInfo(DaemonInfo {
                version: nuage_core::VERSION.to_owned(),
                protocol_version: PROTOCOL_VERSION,
                rclone_version: None,
                pid: std::process::id(),
            })),
            Command::DaemonStop { .. } => Err(Error::not_implemented("stopping the daemon")),
            other => self.engine.handle(other).await,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("NUAGE_LOG")
                .unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .init();

    let args = Args::parse();
    let socket = match args.socket {
        Some(path) => path,
        None => nuage_ipc::socket_path()?,
    };

    let server = match Server::bind(&socket).await {
        Ok(server) => server,
        Err(IpcError::AlreadyRunning(path)) => {
            eprintln!(
                "nuaged is already running (socket {}). Nothing to do.",
                path.display()
            );
            std::process::exit(1);
        }
        Err(e) => {
            return Err(e).with_context(|| format!("cannot listen on {}", socket.display()));
        }
    };

    let engine = Engine::new(ProviderRegistry::with_enabled_providers());
    tracing::info!(
        version = nuage_core::VERSION,
        socket = %server.path().display(),
        providers = ?engine.registry(),
        "nuaged started"
    );
    // TODO(milestone 3): start rclone rcd and remount accounts (Engine::start).

    let (events, _) = broadcast::channel::<Event>(256);
    let daemon = Arc::new(Daemon { engine });

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    tokio::select! {
        result = server.serve(Arc::clone(&daemon), events.clone()) => result?,
        _ = sigterm.recv() => tracing::info!("SIGTERM received"),
        _ = sigint.recv() => tracing::info!("SIGINT received"),
    }

    let _ = events.send(Event::DaemonStopping);
    // TODO(milestone 3): Engine::shutdown (unmount all, then core/quit).
    tracing::info!("nuaged stopped");
    Ok(())
}
