//! Command-line syntax (clap).

use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand};

pub const EXIT_CODES_HELP: &str = "\
Exit codes:
  0  success
  1  error
  2  incorrect usage
  3  daemon unreachable (start it with: systemctl --user start nuaged)
  4  authentication required (run: nuage account reconnect <account>)

Remote paths are written <account>:<path>, e.g. \"Google Drive:Documents/report.odt\".";

/// Nuage: mount your cloud storage accounts as local folders.
#[derive(Debug, Parser)]
#[command(name = "nuage", version, after_help = EXIT_CODES_HELP)]
pub struct Cli {
    /// Print machine-readable JSON instead of text.
    #[arg(long, global = true)]
    pub json: bool,

    /// Daemon socket (default: $XDG_RUNTIME_DIR/nuage/nuaged.sock, then the Flatpak one).
    #[arg(long, global = true, env = "NUAGE_SOCKET", value_name = "PATH")]
    pub socket: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Show the global state, accounts, uploads, quota and cache.
    Status {
        /// Keep running and print every change.
        #[arg(long)]
        watch: bool,
    },
    /// Add, list, rename, reconnect or remove accounts.
    #[command(subcommand)]
    Account(AccountCommand),
    /// Mount an account at its folder.
    Mount { account: String },
    /// Unmount an account.
    Unmount { account: String },
    /// Pause uploads of one account, or of all accounts.
    Pause { account: Option<String> },
    /// Resume uploads of one account, or of all accounts.
    Resume { account: Option<String> },
    /// Look for remote changes now.
    Refresh { account: Option<String> },
    /// List a remote folder.
    Ls {
        /// <account>:<path>
        path: String,
    },
    /// Create a remote folder.
    Mkdir {
        /// <account>:<path>
        path: String,
    },
    /// Move or rename a remote file or folder.
    Mv { source: String, destination: String },
    /// Copy a remote file or folder.
    Cp { source: String, destination: String },
    /// Delete a remote file or folder.
    Rm {
        /// <account>:<path>
        path: String,
        /// Delete folders and their content.
        #[arg(short, long)]
        recursive: bool,
        /// Confirm the deletion (required when not to the trash).
        #[arg(long)]
        yes: bool,
    },
    /// Upload local files to a remote folder.
    Upload {
        #[arg(required = true)]
        local: Vec<PathBuf>,
        /// <account>:<folder>
        destination: String,
    },
    /// Download a remote file or folder.
    Download {
        /// <account>:<path>
        source: String,
        local: PathBuf,
    },
    /// Print a share link for a remote file or folder.
    Link {
        /// <account>:<path>
        path: String,
    },
    /// Exclude remote items from synchronisation.
    #[command(subcommand)]
    Exclude(ExcludeCommand),
    /// Read or change settings (cache limit, cache age…).
    #[command(subcommand)]
    Config(ConfigCommand),
    /// Inspect or stop the daemon.
    #[command(subcommand)]
    Daemon(DaemonCommand),
    /// Keep files available offline (v1.1).
    #[command(subcommand)]
    Pin(PinCommand),
}

#[derive(Debug, Subcommand)]
pub enum AccountCommand {
    /// Connect a new account.
    Add(AccountAdd),
    /// List accounts.
    List,
    /// Rename an account.
    Rename { account: String, name: String },
    /// Mount an account at another folder.
    Move {
        account: String,
        /// The new folder (empty or missing).
        folder: PathBuf,
    },
    /// Sign in again after a token expired or was revoked.
    Reconnect {
        account: String,
        /// OAuth token JSON generated elsewhere with `rclone authorize`.
        #[arg(long, value_name = "JSON", conflicts_with = "token_file")]
        token: Option<String>,
        /// File containing the OAuth token JSON.
        #[arg(long, value_name = "F")]
        token_file: Option<PathBuf>,
        /// Print the sign-in link instead of opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Remove an account from this computer. Nothing is deleted in the cloud.
    Remove {
        account: String,
        /// Confirm the removal (required).
        #[arg(long)]
        yes: bool,
        /// Remove even if uploads are pending (they are lost).
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Args)]
pub struct AccountAdd {
    /// Provider id, e.g. gdrive or local.
    pub provider: String,
    /// Account name (default: the provider name).
    #[arg(long, value_name = "N")]
    pub name: Option<String>,
    /// Folder where the account is mounted (default: ~/Cloud/<name>).
    #[arg(long, value_name = "P")]
    pub mount_point: Option<PathBuf>,
    /// OAuth token JSON generated elsewhere with `rclone authorize`.
    #[arg(long, value_name = "JSON", conflicts_with = "token_file")]
    pub token: Option<String>,
    /// File containing the OAuth token JSON.
    #[arg(long, value_name = "F")]
    pub token_file: Option<PathBuf>,
    /// Your own OAuth client ID.
    #[arg(long, value_name = "ID", requires = "client_secret")]
    pub client_id: Option<String>,
    /// Your own OAuth client secret.
    #[arg(long, value_name = "S", requires = "client_id")]
    pub client_secret: Option<String>,
    /// Print the sign-in URL instead of opening a browser.
    #[arg(long)]
    pub no_browser: bool,
    /// Provider setting, e.g. `--param root=/srv/files` for the local provider.
    #[arg(long = "param", value_name = "KEY=VALUE", value_parser = parse_param)]
    pub params: Vec<(String, String)>,
}

fn parse_param(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((k, v)) if !k.is_empty() => Ok((k.to_owned(), v.to_owned())),
        _ => Err(format!("expected KEY=VALUE, got \"{s}\"")),
    }
}

#[derive(Debug, Subcommand)]
pub enum ExcludeCommand {
    /// Exclude a remote path or a pattern.
    #[command(group(ArgGroup::new("rule").required(true).args(["path", "glob"])))]
    Add {
        account: String,
        /// Path inside the account.
        path: Option<String>,
        /// Pattern applied to the whole account, e.g. '*.tmp'.
        #[arg(long, value_name = "PATTERN")]
        glob: Option<String>,
        /// Confirm the remount if uploads are pending.
        #[arg(long)]
        yes: bool,
    },
    /// Remove an exclusion rule.
    #[command(group(ArgGroup::new("rule").required(true).args(["path", "glob"])))]
    Remove {
        account: String,
        path: Option<String>,
        #[arg(long, value_name = "PATTERN")]
        glob: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// List exclusion rules.
    List { account: Option<String> },
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print one setting, or all of them.
    Get { key: Option<String> },
    /// Change a setting.
    Set { key: String, value: String },
}

#[derive(Debug, Subcommand)]
pub enum DaemonCommand {
    /// Show whether the daemon runs, and its version.
    Status,
    /// Stop the daemon (unmounts every account).
    Stop {
        /// Stop now even if uploads are pending.
        #[arg(long)]
        now: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum PinCommand {
    /// Keep a file or folder (with everything in it) on this computer,
    /// available offline. Returns once it is downloaded.
    Add {
        /// <account>:<path>
        path: String,
    },
    /// Remove the offline copy (the item stays in the cloud).
    Remove {
        /// <account>:<path>
        path: String,
        /// Remove even if local changes are not uploaded yet (they are lost).
        #[arg(long)]
        force: bool,
    },
    /// List items kept offline.
    List { account: Option<String> },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_v1_commands() {
        let ok = [
            "nuage status --json",
            "nuage status --watch",
            "nuage account add gdrive --name Drive --mount-point /tmp/d --token-file t.json --no-browser",
            "nuage account add gdrive --client-id a --client-secret b",
            "nuage account add local --param root=/srv --param x=",
            "nuage account list",
            "nuage account rename Drive Work",
            "nuage account move Drive /tmp/d2",
            "nuage account reconnect Drive",
            "nuage account reconnect Drive --token-file t.json",
            "nuage account reconnect Drive --no-browser",
            "nuage account remove Drive --yes --force",
            "nuage mount Drive",
            "nuage unmount Drive",
            "nuage pause",
            "nuage resume Drive",
            "nuage refresh",
            "nuage ls Drive:",
            "nuage mkdir Drive:new",
            "nuage mv Drive:a Drive:b",
            "nuage cp Drive:a Drive:b",
            "nuage rm Drive:a -r --yes",
            "nuage upload a.txt b.txt Drive:dir",
            "nuage download Drive:a .",
            "nuage link Drive:a",
            "nuage exclude add Drive Archives",
            "nuage exclude add Drive --glob *.tmp",
            "nuage exclude remove Drive --glob *.tmp",
            "nuage exclude list",
            "nuage config get",
            "nuage config set cache-limit 10G",
            "nuage daemon status",
            "nuage daemon stop --now",
            "nuage pin add Drive:a",
            "nuage pin list",
        ];
        for line in ok {
            let args = line.split(' ');
            assert!(Cli::try_parse_from(args).is_ok(), "{line}");
        }

        let bad = [
            "nuage account add gdrive --token x --token-file y",
            "nuage account add gdrive --client-id a",
            "nuage account add local --param =x",
            "nuage exclude add Drive",
            "nuage exclude add Drive path --glob x",
        ];
        for line in bad {
            assert!(Cli::try_parse_from(line.split(' ')).is_err(), "{line}");
        }
    }
}
