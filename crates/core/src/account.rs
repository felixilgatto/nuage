use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Default per-account limit of the local VFS cache: 20 GiB.
pub const DEFAULT_CACHE_LIMIT: u64 = 20 * 1024 * 1024 * 1024;

/// Stable, opaque identifier of an account.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A cloud account connected to Nuage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    /// Identifier of the provider, e.g. `"gdrive"`.
    pub provider_id: String,
    /// Name shown to the user and used on the command line, e.g. `"Google Drive"`.
    pub display_name: String,
    /// Name of the remote in `rclone.conf`.
    pub remote_name: String,
    /// Folder chosen by the user. This is *not* necessarily rclone's own mount
    /// point: from v1.1 on, `nuage-fs` is mounted here on top of a hidden
    /// rclone mount.
    pub mount_point: PathBuf,
    /// Maximum size of the local VFS cache, in bytes.
    pub cache_limit: u64,
}

/// A path on a remote, relative to the root of an account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RemotePath {
    pub account: AccountId,
    /// Slash-separated path, without a leading slash. Empty for the root.
    pub path: String,
}

impl RemotePath {
    pub fn new(account: AccountId, path: impl Into<String>) -> Self {
        let path = path.into();
        Self {
            account,
            path: path.trim_start_matches('/').to_owned(),
        }
    }

    /// Parses the command-line notation `<account>:<path>`.
    ///
    /// Returns `None` when there is no `:` separator or the account is empty.
    pub fn parse(s: &str) -> Option<Self> {
        let (account, path) = s.split_once(':')?;
        if account.is_empty() {
            return None;
        }
        Some(Self::new(AccountId::new(account), path))
    }
}

impl fmt::Display for RemotePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.account, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remote_path() {
        let p = RemotePath::parse("Google Drive:/Documents/report.odt").unwrap();
        assert_eq!(p.account.as_str(), "Google Drive");
        assert_eq!(p.path, "Documents/report.odt");
        assert_eq!(p.to_string(), "Google Drive:Documents/report.odt");
        assert!(RemotePath::parse("no-separator").is_none());
        assert!(RemotePath::parse(":path").is_none());
    }
}
