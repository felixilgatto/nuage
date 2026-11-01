//! `nuaged`: the Nuage daemon.
//!
//! Owns all the state (accounts, mounts, exclusions, pins), drives rclone
//! through the engine, and serves the GTK app, the tray and the CLI over a
//! Unix socket. One daemon per user: a second instance detects the first
//! through the socket and exits.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;

use nuage_core::{Command, DaemonInfo, Error, Event, Reply};
use nuage_engine::{Engine, EngineConfig, Paths, ProviderRegistry, StopPlan};
use nuage_ipc::{Handler, IpcError, PROTOCOL_VERSION, Server};

/// Nuage daemon: mounts cloud accounts with rclone and serves Nuage clients.
#[derive(Debug, Parser)]
#[command(name = "nuaged", version)]
struct Args {
    /// Socket to listen on (default: $XDG_RUNTIME_DIR/nuage/nuaged.sock).
    #[arg(long, env = "NUAGE_SOCKET", value_name = "PATH")]
    socket: Option<PathBuf>,

    /// rclone binary (default: /app/bin/rclone in the Flatpak, else rclone from PATH).
    #[arg(long, env = "NUAGE_RCLONE", value_name = "PATH")]
    rclone: Option<PathBuf>,
}

struct Daemon {
    engine: Engine,
    stop: mpsc::Sender<StopPlan>,
}

impl Handler for Daemon {
    async fn handle(&self, command: Command) -> Result<Reply, Error> {
        tracing::debug!(command = command_name(&command), "command");
        match command {
            Command::DaemonInfo => Ok(Reply::DaemonInfo(DaemonInfo {
                version: nuage_core::VERSION.to_owned(),
                protocol_version: PROTOCOL_VERSION,
                rclone_version: self.engine.rclone_version().await,
                pid: std::process::id(),
            })),
            Command::DaemonStop { now } => {
                let plan = self.engine.stop_plan(now).await?;
                let _ = self.stop.send(plan).await;
                Ok(Reply::Done)
            }
            other => self.engine.handle(other).await,
        }
    }
}

/// Name of a command for the log. Commands may carry tokens and secrets,
/// so they are never logged in full.
fn command_name(command: &Command) -> String {
    serde_json::to_value(command)
        .ok()
        .and_then(|v| v.get("command")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The rclone binary to run.
fn find_rclone(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let bundled = Path::new("/app/bin/rclone");
    if bundled.exists() {
        return Ok(bundled.to_owned());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path)
        .map(|dir| dir.join("rclone"))
        .find(|p| p.is_file())
        .context(
            "rclone was not found. Install rclone, or point NUAGE_RCLONE at the rclone binary",
        )?;
    if found.starts_with("/snap") {
        tracing::warn!(
            path = %found.display(),
            "the snap of rclone cannot mount folders; install rclone from https://rclone.org/install/ and set NUAGE_RCLONE"
        );
    }
    Ok(found)
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

    let config = EngineConfig {
        paths: Paths::from_env()?,
        rclone_path: find_rclone(args.rclone)?,
    };
    tracing::info!(
        version = nuage_core::VERSION,
        socket = %server.path().display(),
        rclone = %config.rclone_path.display(),
        config = %config.paths.config_dir.display(),
        "nuaged starting"
    );
    let engine = Engine::new(ProviderRegistry::with_enabled_providers(), config)?;
    engine.start().await.context("cannot start rclone")?;
    tracing::info!(providers = ?engine.registry(), "nuaged started");

    let (stop, mut stop_requests) = mpsc::channel(1);
    let events = engine.events();
    let daemon = Arc::new(Daemon { engine, stop });

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let serve = server.serve(Arc::clone(&daemon), events.clone());
    tokio::pin!(serve);

    let plan = tokio::select! {
        result = &mut serve => { result?; StopPlan::Now }
        _ = sigterm.recv() => { tracing::info!("SIGTERM received"); StopPlan::Now }
        _ = sigint.recv() => { tracing::info!("SIGINT received"); StopPlan::Now }
        Some(plan) = stop_requests.recv() => plan,
    };
    if plan == StopPlan::AfterUploads {
        tracing::info!("stopping once the uploads are done");
        tokio::select! {
            () = daemon.engine.wait_for_uploads() => {}
            result = &mut serve => result?,
            _ = sigterm.recv() => tracing::info!("SIGTERM received"),
            _ = sigint.recv() => tracing::info!("SIGINT received"),
            Some(StopPlan::Now) = stop_requests.recv() => {}
        }
    }

    let _ = events.send(Event::DaemonStopping);
    if let Err(e) = daemon.engine.shutdown().await {
        tracing::error!(error = %e, "shutdown failed");
    }
    tracing::info!("nuaged stopped");
    Ok(())
}
