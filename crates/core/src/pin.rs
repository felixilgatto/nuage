//! Offline pinning (v1.1): files and folders kept on this computer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AccountId;

/// A file or folder kept available offline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    pub account: AccountId,
    pub path: String,
    pub kind: PinKind,
    pub pinned_at: DateTime<Utc>,
    #[serde(default)]
    pub state: PinState,
    /// Bytes kept on this computer.
    #[serde(default)]
    pub size: u64,
    /// Local changes not uploaded yet (offline, paused, being written).
    #[serde(default)]
    pub pending: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    File,
    FolderRecursive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    Downloading,
    #[default]
    Available,
    ModifiedLocally,
    Conflict,
    Error,
}
