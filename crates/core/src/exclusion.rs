use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{AccountId, RemotePath};

/// A rule excluding remote items from synchronisation for one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exclusion {
    pub account: AccountId,
    pub rule: ExclusionRule,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ExclusionRule {
    /// A specific remote file or folder.
    Path(RemotePath),
    /// A pattern applied to the whole account, e.g. `*.tmp` or `node_modules/`.
    Glob(String),
}
