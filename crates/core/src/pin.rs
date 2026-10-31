//! Offline pinning (v1.1). The types exist now so that the protocol and the
//! account configuration do not change when the feature lands.

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinKind {
    File,
    FolderRecursive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinState {
    Downloading,
    Available,
    ModifiedLocally,
    Conflict,
    Error,
}
