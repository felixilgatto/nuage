//! Nuage's own configuration (`config.toml`): accounts, pause state and
//! settings. rclone's remotes live in `rclone.conf`.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use nuage_core::{Account, DEFAULT_CACHE_LIMIT, Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, rename = "account")]
    pub accounts: Vec<AccountRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Cache limit given to new accounts, in bytes.
    pub default_cache_limit: u64,
    /// Maximum age of cached files, in seconds. `None`: the provider's
    /// recommendation.
    pub cache_max_age_secs: Option<u64>,
    pub quit_with_pending: QuitBehavior,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_cache_limit: DEFAULT_CACHE_LIMIT,
            cache_max_age_secs: None,
            quit_with_pending: QuitBehavior::default(),
        }
    }
}

/// What "Quit" and `nuage daemon stop` do while uploads are pending.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuitBehavior {
    /// Stop once the uploads are done.
    #[default]
    FinishInBackground,
    /// Refuse, and let the user choose (`--now`).
    Ask,
    /// Stop at once; uploads resume at the next start.
    QuitNow,
}

impl QuitBehavior {
    pub const ALL: [QuitBehavior; 3] = [Self::FinishInBackground, Self::Ask, Self::QuitNow];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FinishInBackground => "finish-in-background",
            Self::Ask => "ask",
            Self::QuitNow => "quit-now",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.as_str() == s)
    }
}

/// An account and its persistent state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountRecord {
    #[serde(flatten)]
    pub account: Account,
    /// Uploads are paused. Survives daemon restarts.
    #[serde(default)]
    pub paused: bool,
    /// Mounted when the daemon starts; cleared by `nuage unmount`.
    #[serde(default = "yes")]
    pub auto_mount: bool,
}

fn yes() -> bool {
    true
}

impl AppConfig {
    /// Reads `path`; a missing file is an empty configuration.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| {
                Error::Internal(format!(
                    "cannot read {}: {e}. Fix or remove the file, then restart nuaged.",
                    path.display()
                ))
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes `path` atomically, readable by the user only.
    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| Error::Internal(format!("cannot serialise the configuration: {e}")))?;
        let dir = path
            .parent()
            .ok_or_else(|| Error::Internal("configuration path has no parent".to_owned()))?;
        std::fs::create_dir_all(dir)?;
        let tmp = path.with_extension("toml.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::AccountId;

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nuage/config.toml");
        assert_eq!(AppConfig::load(&path).unwrap(), AppConfig::default());

        let config = AppConfig {
            settings: Settings {
                quit_with_pending: QuitBehavior::Ask,
                ..Settings::default()
            },
            accounts: vec![AccountRecord {
                account: Account {
                    id: AccountId::new("0a1b2c3d"),
                    provider_id: "local".into(),
                    display_name: "Local folder".into(),
                    remote_name: "nuage-0a1b2c3d".into(),
                    mount_point: "/home/me/Cloud/Local folder".into(),
                    cache_limit: 1 << 30,
                },
                paused: true,
                auto_mount: false,
            }],
        };
        config.save(&path).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), config);
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
