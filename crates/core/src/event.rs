//! Notifications pushed by the daemon to subscribed clients.

use serde::{Deserialize, Serialize};

use crate::{Account, AccountId, PinState, Quota, StatusReport, SyncState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// The aggregated or per-account status changed.
    StatusChanged {
        status: StatusReport,
    },
    AccountAdded {
        account: Account,
    },
    AccountUpdated {
        account: Account,
    },
    AccountRemoved {
        account: AccountId,
    },
    AccountStateChanged {
        account: AccountId,
        state: SyncState,
    },
    MountChanged {
        account: AccountId,
        mounted: bool,
    },
    QuotaUpdated {
        account: AccountId,
        quota: Quota,
    },
    /// An upload failed; `message` says what to do.
    UploadFailed {
        account: AccountId,
        path: String,
        message: String,
    },
    /// The account's token expired or was revoked: it must be reconnected.
    AuthRequired {
        account: AccountId,
    },
    CacheFull {
        account: AccountId,
    },
    /// State of a pinned item changed (v1.1).
    PinStateChanged {
        account: AccountId,
        path: String,
        state: PinState,
    },
    /// The daemon is shutting down.
    DaemonStopping,
}
