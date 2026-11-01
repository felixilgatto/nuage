//! `nuage`: the Nuage command-line interface.
//!
//! A client of `nuaged`. Never prompts: confirmations are flags (`--yes`).
//! Every command accepts `--json`; exit codes are documented in
//! [`args::EXIT_CODES_HELP`].

mod args;

use std::process::ExitCode;

use clap::Parser;
use serde_json::json;

use nuage_core::{
    AccountAddArgs, AccountId, Command, Event, ExclusionRule, Quota, RemotePath, Reply,
    StatusReport, SyncState,
};
use nuage_ipc::{Client, IpcError};

use args::{AccountCommand, Cli, Commands, ConfigCommand, DaemonCommand, ExcludeCommand};

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

fn usage(message: impl Into<String>) -> Failure {
    Failure {
        code: "usage",
        message: message.into(),
        exit: exit::USAGE,
    }
}

fn unexpected() -> Failure {
    IpcError::UnexpectedMessage("daemon").into()
}

async fn connect(cli: &Cli) -> Result<Client, Failure> {
    let name = format!("nuage {}", nuage_core::VERSION);
    let paths = match &cli.socket {
        Some(path) => vec![path.clone()],
        None => nuage_ipc::candidate_socket_paths()?,
    };
    Ok(Client::connect_any(&paths, &name).await?)
}

/// Sends one command on a new connection.
async fn request(cli: &Cli, command: Command) -> Result<Reply, Failure> {
    let mut client = connect(cli).await?;
    Ok(client.request(command).await??)
}

/// Prints the outcome of a command that returns nothing.
fn done(cli: &Cli, message: &str) {
    if cli.json {
        print_json(&json!({ "ok": true }));
    } else {
        println!("{message}");
    }
}

fn account(name: &str) -> AccountId {
    AccountId::new(name)
}

async fn run(cli: &Cli) -> Result<(), Failure> {
    match &cli.command {
        Commands::Status { watch } => status(cli, *watch).await,
        Commands::Account(command) => account_command(cli, command).await,
        Commands::Mount { account: name } => {
            request(
                cli,
                Command::Mount {
                    account: account(name),
                },
            )
            .await?;
            done(cli, &format!("Mounted \"{name}\"."));
            Ok(())
        }
        Commands::Unmount { account: name } => {
            request(
                cli,
                Command::Unmount {
                    account: account(name),
                },
            )
            .await?;
            done(
                cli,
                &format!(
                    "Unmounted \"{name}\". It stays unmounted until `nuage mount \"{name}\"`."
                ),
            );
            Ok(())
        }
        Commands::Pause { account: name } => {
            let target = name.as_deref().map(account);
            request(cli, Command::Pause { account: target }).await?;
            let what = name
                .as_deref()
                .map_or("all accounts".to_owned(), |n| format!("\"{n}\""));
            done(
                cli,
                &format!(
                    "Uploads of {what} are paused. Files written meanwhile wait in the cache until `nuage resume`."
                ),
            );
            Ok(())
        }
        Commands::Resume { account: name } => {
            let target = name.as_deref().map(account);
            request(cli, Command::Resume { account: target }).await?;
            let what = name
                .as_deref()
                .map_or("all accounts".to_owned(), |n| format!("\"{n}\""));
            done(cli, &format!("Uploads of {what} are resumed."));
            Ok(())
        }
        Commands::Refresh { account: name } => {
            let target = name.as_deref().map(account);
            request(cli, Command::Refresh { account: target }).await?;
            done(
                cli,
                "Refreshed: remote changes show up in the mounted folders.",
            );
            Ok(())
        }
        Commands::Config(command) => {
            let command = match command {
                ConfigCommand::Get { key } => Command::ConfigGet { key: key.clone() },
                ConfigCommand::Set { key, value } => Command::ConfigSet {
                    key: key.clone(),
                    value: value.clone(),
                },
            };
            let is_set = matches!(command, Command::ConfigSet { .. });
            let Reply::Config(entries) = request(cli, command).await? else {
                return Err(unexpected());
            };
            if cli.json {
                let map: serde_json::Map<String, serde_json::Value> = entries
                    .into_iter()
                    .map(|e| (e.key, e.value.into()))
                    .collect();
                print_json(&map);
            } else {
                for e in &entries {
                    println!("{} = {}", e.key, e.value);
                }
                if is_set && entries.iter().any(|e| e.key.contains("cache")) {
                    println!(
                        "Mounted accounts use the new value once remounted (nuage unmount, then nuage mount)."
                    );
                }
            }
            Ok(())
        }
        Commands::Daemon(DaemonCommand::Status) => {
            let Reply::DaemonInfo(info) = request(cli, Command::DaemonInfo).await? else {
                return Err(unexpected());
            };
            if cli.json {
                print_json(&info);
            } else {
                println!(
                    "Nuage daemon {} is running (PID {}).",
                    info.version, info.pid
                );
                match &info.rclone_version {
                    Some(v) => println!("rclone {v}"),
                    None => println!("rclone is not running."),
                }
            }
            Ok(())
        }
        Commands::Daemon(DaemonCommand::Stop { now }) => {
            request(cli, Command::DaemonStop { now: *now }).await?;
            done(
                cli,
                "The Nuage daemon is stopping. Pending uploads are finished first unless --now was given.",
            );
            Ok(())
        }
        Commands::Exclude(command) => exclude_command(cli, command).await,
        other => Err(not_implemented(other.name())),
    }
}

fn exclusion_rule(
    account_name: &str,
    path: &Option<String>,
    glob: &Option<String>,
) -> ExclusionRule {
    match (path, glob) {
        (_, Some(glob)) => ExclusionRule::Glob(glob.clone()),
        (Some(path), None) => {
            ExclusionRule::Path(RemotePath::new(account(account_name), path.clone()))
        }
        // clap requires one of them.
        (None, None) => unreachable!("path or --glob"),
    }
}

fn describe_rule(rule: &ExclusionRule) -> String {
    match rule {
        ExclusionRule::Path(p) => format!("/{}", p.path),
        ExclusionRule::Glob(g) => format!("pattern {g}"),
    }
}

async fn exclude_command(cli: &Cli, command: &ExcludeCommand) -> Result<(), Failure> {
    match command {
        ExcludeCommand::Add {
            account: name,
            path,
            glob,
            yes,
        } => {
            let rule = exclusion_rule(name, path, glob);
            let what = describe_rule(&rule);
            request(
                cli,
                Command::ExcludeAdd {
                    account: account(name),
                    rule,
                    force: *yes,
                },
            )
            .await?;
            done(
                cli,
                &format!(
                    "Excluded {what} from \"{name}\": it is no longer downloaded nor uploaded, and stays intact in the cloud."
                ),
            );
            Ok(())
        }
        ExcludeCommand::Remove {
            account: name,
            path,
            glob,
            yes,
        } => {
            let rule = exclusion_rule(name, path, glob);
            let what = describe_rule(&rule);
            request(
                cli,
                Command::ExcludeRemove {
                    account: account(name),
                    rule,
                    force: *yes,
                },
            )
            .await?;
            done(cli, &format!("{what} of \"{name}\" is synchronised again."));
            Ok(())
        }
        ExcludeCommand::List { account: name } => {
            let Reply::Exclusions(list) = request(
                cli,
                Command::ExcludeList {
                    account: name.as_deref().map(account),
                },
            )
            .await?
            else {
                return Err(unexpected());
            };
            if cli.json {
                print_json(&list);
                return Ok(());
            }
            let accounts = match request(cli, Command::AccountList).await? {
                Reply::Accounts(a) => a,
                _ => return Err(unexpected()),
            };
            let name_of = |id: &AccountId| {
                accounts
                    .iter()
                    .find(|a| &a.id == id)
                    .map_or_else(|| id.to_string(), |a| a.display_name.clone())
            };
            if list.is_empty() {
                println!("Nothing is excluded.");
            }
            let mut current = None;
            for e in &list {
                if current != Some(&e.account) {
                    println!("{}:", name_of(&e.account));
                    current = Some(&e.account);
                }
                let note = if e.default { "  (default)" } else { "" };
                println!("  {}{note}", describe_rule(&e.rule));
            }
            Ok(())
        }
    }
}

async fn account_command(cli: &Cli, command: &AccountCommand) -> Result<(), Failure> {
    match command {
        AccountCommand::Add(add) => {
            let token = read_token(&add.token, &add.token_file)?;
            // The daemon does not share our working directory.
            let mount_point = add
                .mount_point
                .as_deref()
                .map(std::path::absolute)
                .transpose()
                .map_err(|e| usage(format!("Invalid mount folder: {e}")))?;
            let args = AccountAddArgs {
                provider: add.provider.clone(),
                name: add.name.clone(),
                mount_point,
                token,
                client_id: add.client_id.clone(),
                client_secret: add.client_secret.clone(),
                no_browser: add.no_browser,
                params: add.params.iter().cloned().collect(),
            };
            let mut client = connect(cli).await?;
            let reply = client.request(Command::AccountAdd(args)).await??;
            let a = finish_sign_in(cli, client, reply, add.no_browser).await?;
            if cli.json {
                print_json(&a);
            } else {
                println!(
                    "Added \"{}\" ({}), mounted at {}.",
                    a.display_name,
                    a.provider_id,
                    a.mount_point.display()
                );
            }
            Ok(())
        }
        AccountCommand::List => {
            let Reply::Accounts(accounts) = request(cli, Command::AccountList).await? else {
                return Err(unexpected());
            };
            if cli.json {
                print_json(&accounts);
            } else if accounts.is_empty() {
                println!("No accounts yet. Add one with: nuage account add <provider>");
            } else {
                for a in &accounts {
                    println!(
                        "{}\t{}\t{}",
                        a.display_name,
                        a.provider_id,
                        a.mount_point.display()
                    );
                }
            }
            Ok(())
        }
        AccountCommand::Rename {
            account: name,
            name: new,
        } => {
            let Reply::Account(a) = request(
                cli,
                Command::AccountRename {
                    account: account(name),
                    name: new.clone(),
                },
            )
            .await?
            else {
                return Err(unexpected());
            };
            if cli.json {
                print_json(&a);
            } else {
                println!("Renamed \"{name}\" to \"{}\".", a.display_name);
            }
            Ok(())
        }
        AccountCommand::Reconnect {
            account: name,
            token,
            token_file,
            no_browser,
        } => {
            let token = read_token(token, token_file)?;
            let mut client = connect(cli).await?;
            let reply = client
                .request(Command::AccountReconnect {
                    account: account(name),
                    token,
                })
                .await??;
            let a = finish_sign_in(cli, client, reply, *no_browser).await?;
            if cli.json {
                print_json(&a);
            } else {
                println!("Reconnected \"{}\".", a.display_name);
            }
            Ok(())
        }
        AccountCommand::Remove {
            account: name,
            yes,
            force,
        } => {
            if !yes {
                return Err(usage(format!(
                    "Removing \"{name}\" deletes its local cache and settings (nothing is deleted in the cloud). Add --yes to confirm."
                )));
            }
            request(
                cli,
                Command::AccountRemove {
                    account: account(name),
                    force: *force,
                },
            )
            .await?;
            done(
                cli,
                &format!(
                    "Removed \"{name}\" from this computer. Nothing was deleted in the cloud."
                ),
            );
            Ok(())
        }
    }
}

fn read_token(
    token: &Option<String>,
    token_file: &Option<std::path::PathBuf>,
) -> Result<Option<String>, Failure> {
    Ok(match (token, token_file) {
        (Some(token), _) => Some(token.clone()),
        (None, Some(file)) => Some(
            std::fs::read_to_string(file)
                .map_err(|e| usage(format!("Cannot read {}: {e}", file.display())))?
                .trim()
                .to_owned(),
        ),
        (None, None) => None,
    })
}

/// Whether a web browser can be opened here.
fn can_open_browser() -> bool {
    let has = |v: &str| std::env::var_os(v).is_some_and(|x| !x.is_empty());
    (has("DISPLAY") || has("WAYLAND_DISPLAY")) && !has("SSH_CONNECTION")
}

/// When the daemon asks for a sign-in in a browser: shows the link (on
/// stderr, so that `--json` output stays one document), opens it if it can,
/// and waits. Ctrl-C cancels the sign-in.
async fn finish_sign_in(
    cli: &Cli,
    mut client: Client,
    reply: Reply,
    no_browser: bool,
) -> Result<nuage_core::Account, Failure> {
    let sign_in = match reply {
        Reply::Account(a) => return Ok(a),
        Reply::SignIn(sign_in) => sign_in,
        _ => return Err(unexpected()),
    };
    eprintln!(
        "To finish, sign in with your web browser at:\n\n    {}\n",
        sign_in.url
    );
    if !no_browser && can_open_browser() {
        let opened = std::process::Command::new("xdg-open")
            .arg(&sign_in.url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if opened {
            eprintln!("Your browser should open. If it does not, open the link above.");
        }
    } else {
        eprintln!(
            "This page is served by the machine running Nuage, on port 53682.\n\
             If that is a server, open a tunnel from your computer first:\n\n    \
             ssh -L 53682:localhost:53682 <server>\n\n\
             then open the link in your computer's browser."
        );
    }
    eprintln!("Waiting for the sign-in (Ctrl-C to cancel)…");

    let session = sign_in.session.clone();
    tokio::select! {
        reply = client.request(Command::SignInWait { session: sign_in.session }) => {
            match reply?? {
                Reply::Account(a) => Ok(a),
                _ => Err(unexpected()),
            }
        }
        _ = tokio::signal::ctrl_c() => {
            let _ = request(cli, Command::SignInCancel { session }).await;
            Err(nuage_core::Error::AuthRequired("Sign-in cancelled.".to_owned()).into())
        }
    }
}

async fn status(cli: &Cli, watch: bool) -> Result<(), Failure> {
    let mut client = connect(cli).await?;
    let Reply::Status(report) = client.request(Command::Status).await?? else {
        return Err(unexpected());
    };
    let version = client.daemon_version().to_owned();
    if !watch {
        if cli.json {
            print_json(&report);
        } else {
            print_status(&report, &version);
        }
        return Ok(());
    }

    // One JSON object per line with --json, so that scripts can stream it.
    let show = |report: &StatusReport| {
        if cli.json {
            println!("{}", serde_json::to_string(report).expect("serialisable"));
        } else {
            print_status(report, &version);
            println!();
        }
    };
    show(&report);
    let mut events = client.subscribe().await?;
    while let Some(event) = events.next_event().await? {
        match event {
            Event::StatusChanged { status } => show(&status),
            Event::DaemonStopping => {
                if !cli.json {
                    println!("The Nuage daemon stopped.");
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

fn print_json(value: &impl serde::Serialize) {
    // Serialising our own types cannot fail.
    println!(
        "{}",
        serde_json::to_string_pretty(value).expect("serialisable")
    );
}

/// Bytes in a human-readable form (`1.5 GB`, powers of 1024).
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
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
        } => {
            if *failed_files > 0 {
                format!("⚠ Problem ({failed_files} failed): {message}")
            } else {
                format!("⚠ Problem: {message}")
            }
        }
        SyncState::Stopped => "⏸ Not mounted".to_owned(),
        SyncState::Offline => "⚠ Offline".to_owned(),
    }
}

fn quota_line(quota: &Quota) -> Option<String> {
    let mut parts = Vec::new();
    match (quota.used, quota.total) {
        (Some(used), Some(total)) => parts.push(format!(
            "{} of {} used ({:.0}%)",
            human_size(used),
            human_size(total),
            quota.used_percent().unwrap_or(0.0)
        )),
        (Some(used), None) => parts.push(format!("{} used", human_size(used))),
        _ => {}
    }
    if let Some(free) = quota.free {
        parts.push(format!("{} free", human_size(free)));
    }
    if let Some(trashed) = quota.trashed.filter(|t| *t > 0) {
        parts.push(format!("{} in the trash", human_size(trashed)));
    }
    (!parts.is_empty()).then(|| parts.join(", "))
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
        println!("\n{} ({})", account.display_name, account.provider_id);
        println!("  {}", state_line(&status.state));
        if status.mounted {
            println!("  Folder: {}", account.mount_point.display());
        } else {
            println!(
                "  Folder: {} (not mounted; run: nuage mount \"{}\")",
                account.mount_point.display(),
                account.display_name
            );
        }
        if let Some(line) = status.quota.as_ref().and_then(quota_line) {
            println!("  Storage: {line}");
        }
        println!(
            "  Cache: {} of {}",
            human_size(status.cache_used),
            human_size(account.cache_limit)
        );
    }
    if !report.config_encrypted {
        println!(
            "\nNote: rclone.conf is not encrypted (no keyring); it is readable by your user only."
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
