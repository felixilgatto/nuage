//! Requests sent by clients to the daemon, and their replies.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{
    Account, AccountId, AuthKind, Capabilities, Entry, Exclusion, ExclusionRule, Pin, PinKind,
    Quota, RemotePath, SyncState,
};

/// A request from a client (CLI, GTK app, tray) to the daemon.
///
/// Everything the graphical interface does goes through these commands, so
/// everything is also possible from the command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    // Daemon and global state
    /// Global and per-account state.
    Status,
    DaemonInfo,
    DaemonStop {
        /// Stop immediately even if uploads are pending.
        now: bool,
    },
    ListProviders,

    // Accounts
    AccountAdd(AccountAddArgs),
    AccountList,
    AccountRename {
        account: AccountId,
        name: String,
    },
    AccountReconnect {
        account: AccountId,
    },
    AccountRemove {
        account: AccountId,
        /// Remove even if uploads are pending.
        force: bool,
    },

    // Mounts and uploads
    Mount {
        account: AccountId,
    },
    Unmount {
        account: AccountId,
    },
    /// Pause uploads of one account, or of all accounts when `None`.
    Pause {
        account: Option<AccountId>,
    },
    Resume {
        account: Option<AccountId>,
    },
    Refresh {
        account: Option<AccountId>,
    },

    // Files
    List {
        path: RemotePath,
    },
    Mkdir {
        path: RemotePath,
    },
    Move {
        from: RemotePath,
        to: RemotePath,
    },
    Copy {
        from: RemotePath,
        to: RemotePath,
    },
    Delete {
        path: RemotePath,
        recursive: bool,
    },
    Upload {
        local: PathBuf,
        to: RemotePath,
    },
    Download {
        from: RemotePath,
        local: PathBuf,
    },
    PublicLink {
        path: RemotePath,
    },

    // Exclusions
    ExcludeAdd {
        account: AccountId,
        rule: ExclusionRule,
    },
    ExcludeRemove {
        account: AccountId,
        rule: ExclusionRule,
    },
    ExcludeList {
        account: Option<AccountId>,
    },

    // Settings
    ConfigGet {
        key: Option<String>,
    },
    ConfigSet {
        key: String,
        value: String,
    },

    // Offline pinning (v1.1)
    PinAdd {
        path: RemotePath,
        kind: PinKind,
    },
    PinRemove {
        path: RemotePath,
    },
    PinList {
        account: Option<AccountId>,
    },
}

/// Arguments of [`Command::AccountAdd`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAddArgs {
    pub provider: String,
    pub name: Option<String>,
    pub mount_point: Option<PathBuf>,
    /// OAuth token JSON generated elsewhere.
    pub token: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    /// Print the sign-in URL instead of opening a browser.
    pub no_browser: bool,
}

/// Successful reply to a [`Command`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum Reply {
    /// The command succeeded and has nothing to return.
    Done,
    Status(StatusReport),
    DaemonInfo(DaemonInfo),
    Providers(Vec<ProviderInfo>),
    Account(Account),
    Accounts(Vec<Account>),
    Entries(Vec<Entry>),
    Link(String),
    Exclusions(Vec<Exclusion>),
    Config(Vec<ConfigEntry>),
    Pins(Vec<Pin>),
}

/// Global state, as shown by the tray icon and `nuage status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusReport {
    /// Aggregated state of all accounts.
    pub state: SyncState,
    pub accounts: Vec<AccountStatus>,
    /// Total size of the local caches of all accounts, in bytes.
    pub cache_used: u64,
    /// Whether `rclone.conf` is encrypted with a password from the keyring.
    /// `false` on a headless machine without a keyring (file in mode 0600).
    pub config_encrypted: bool,
}

impl StatusReport {
    /// State of a daemon without any account.
    pub fn empty() -> Self {
        Self {
            state: SyncState::UpToDate,
            accounts: Vec::new(),
            cache_used: 0,
            config_encrypted: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountStatus {
    pub account: Account,
    pub state: SyncState,
    pub mounted: bool,
    pub quota: Option<Quota>,
    /// Size of the account's local cache, in bytes.
    pub cache_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonInfo {
    pub version: String,
    pub protocol_version: u32,
    pub rclone_version: Option<String>,
    pub pid: u32,
}

/// Description of a provider, for clients that do not link the providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub display_name: String,
    pub icon_name: String,
    pub auth: AuthKind,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trip() {
        let commands = [
            Command::Status,
            Command::Pause { account: None },
            Command::Mount {
                account: AccountId::new("Google Drive"),
            },
            Command::AccountAdd(AccountAddArgs {
                provider: "local".into(),
                ..AccountAddArgs::default()
            }),
            Command::ExcludeAdd {
                account: AccountId::new("Drive"),
                rule: ExclusionRule::Glob("*.tmp".into()),
            },
        ];
        for command in commands {
            let json = serde_json::to_string(&command).unwrap();
            let back: Command = serde_json::from_str(&json).unwrap();
            assert_eq!(back, command);
        }
        assert_eq!(
            serde_json::to_string(&Command::Status).unwrap(),
            r#"{"command":"status"}"#
        );
    }

    #[test]
    fn reply_round_trip() {
        let reply = Reply::Status(StatusReport::empty());
        let json = serde_json::to_string(&reply).unwrap();
        assert_eq!(serde_json::from_str::<Reply>(&json).unwrap(), reply);
    }
}
