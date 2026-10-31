//! `nuage`: the Nuage command-line interface.
//!
//! A client of `nuaged`. Never prompts: confirmations are flags (`--yes`).
//! Every command accepts `--json`; exit codes are documented in
//! [`args::EXIT_CODES_HELP`].

mod args;

use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use nuage_core::{Command, Reply, StatusReport, SyncState};
use nuage_ipc::{Client, IpcError};

use args::{Cli, Commands};

/// Documented exit codes.
mod exit {
    pub const SUCCESS: u8 = 0;
    pub const ERROR: u8 = 1;
    pub const USAGE: u8 = 2;
    pub const UNREACHABLE: u8 = 3;
    pub const AUTH_REQUIRED: u8 = 4;
}

/// A failure, with its machine-readable code and exit code.
#[derive(Debug)]
struct Failure {
    code: &'static str,
    message: String,
    exit: u8,
}

impl From<IpcError> for Failure {
    fn from(e: IpcError) -> Self {
        let (code, exit) = match &e {
            IpcError::Unreachable { .. } | IpcError::NoRuntimeDir => {
                ("daemon_unreachable", exit::UNREACHABLE)
            }
            IpcError::ProtocolMismatch(_) => ("protocol_mismatch", exit::ERROR),
            _ => ("ipc", exit::ERROR),
        };
        Failure {
            code,
            message: e.to_string(),
            exit,
        }
    }
}

impl From<nuage_core::Error> for Failure {
    fn from(e: nuage_core::Error) -> Self {
        let exit = match e {
            nuage_core::Error::AuthRequired(_) => exit::AUTH_REQUIRED,
            nuage_core::Error::InvalidArgument(_) => exit::USAGE,
            _ => exit::ERROR,
        };
        Failure {
            code: e.code(),
            message: e.to_string(),
            exit,
        }
    }
}

fn not_implemented(what: &str) -> Failure {
    Failure {
        code: "not_implemented",
        message: format!("`nuage {what}` is not implemented yet."),
        exit: exit::ERROR,
    }
}

async fn connect(cli: &Cli) -> Result<Client, Failure> {
    let name = format!("nuage {}", nuage_core::VERSION);
    let paths = match &cli.socket {
        Some(path) => vec![path.clone()],
        None => nuage_ipc::candidate_socket_paths()?,
    };
    Ok(Client::connect_any(&paths, &name).await?)
}

async fn run(cli: &Cli) -> Result<(), Failure> {
    match &cli.command {
        Commands::Status { watch: false } => {
            let mut client = connect(cli).await?;
            match client.request(Command::Status).await?? {
                Reply::Status(report) => {
                    if cli.json {
                        print_json(&report);
                    } else {
                        print_status(&report, client.daemon_version());
                    }
                    Ok(())
                }
                _ => Err(IpcError::UnexpectedMessage("daemon").into()),
            }
        }
        Commands::Status { watch: true } => Err(not_implemented("status --watch")),
        other => Err(not_implemented(other.name())),
    }
}

fn print_json(value: &impl serde::Serialize) {
    // Serialising our own types cannot fail.
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialisable")
    );
}

fn state_line(state: &SyncState) -> String {
    match state {
        SyncState::UpToDate => "✓ Up to date".to_owned(),
        SyncState::Syncing { uploads, queued } => {
            format!("↻ Syncing ({uploads} uploading, {queued} queued)")
        }
        SyncState::Paused { queued } => format!("⏸ Paused ({queued} waiting)"),
        SyncState::Error {
            message,
            failed_files,
        } => format!("⚠ Problem ({failed_files} failed): {message}"),
        SyncState::Stopped => "⏸ Stopped".to_owned(),
        SyncState::Offline => "⚠ Offline".to_owned(),
    }
}

fn print_status(report: &StatusReport, daemon_version: &str) {
    println!("Nuage daemon {daemon_version} is running.");
    println!("Status: {}", state_line(&report.state));
    if report.accounts.is_empty() {
        println!("No accounts yet. Add one with: nuage account add <provider>");
        return;
    }
    for status in &report.accounts {
        let account = &status.account;
        let mount = if status.mounted {
            format!("mounted at {}", account.mount_point.display())
        } else {
            "not mounted".to_owned()
        };
        println!(
            "  {}: {} — {}",
            account.display_name,
            state_line(&status.state),
            mount
        );
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: cannot start the async runtime: {e}");
            return ExitCode::from(exit::ERROR);
        }
    };

    match runtime.block_on(run(&cli)) {
        Ok(()) => ExitCode::from(exit::SUCCESS),
        Err(failure) => {
            if cli.json {
                print_json(&json!({
                    "error": { "code": failure.code, "message": failure.message }
                }));
            } else {
                eprintln!("Error: {}", failure.message);
            }
            ExitCode::from(failure.exit)
        }
    }
}
