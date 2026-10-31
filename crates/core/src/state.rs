use serde::{Deserialize, Serialize};

/// Synchronisation state of an account, or of all accounts once aggregated.
///
/// "Up to date" means that no local write is waiting to be uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SyncState {
    UpToDate,
    Syncing {
        /// Files currently being uploaded.
        uploads: u32,
        /// Files waiting in the upload queue.
        queued: u32,
    },
    Paused {
        /// Files waiting until uploads are resumed.
        queued: u32,
    },
    Error {
        /// Human-readable description, with the action to take.
        message: String,
        /// Number of files that failed to upload.
        failed_files: u32,
    },
    Stopped,
    Offline,
}

impl SyncState {
    /// Aggregates the states of several accounts into the global state shown
    /// by the tray icon and `nuage status`.
    ///
    /// Priority: error > syncing > paused/stopped > offline > up to date.
    /// Upload counters are summed. An empty set of accounts is up to date.
    pub fn aggregate<'a>(states: impl IntoIterator<Item = &'a SyncState>) -> SyncState {
        let mut errors = Vec::new();
        let mut failed = 0;
        let (mut uploads, mut queued, mut paused_queued) = (0, 0, 0);
        let (mut syncing, mut paused, mut stopped, mut offline) = (false, false, false, false);

        for state in states {
            match state {
                SyncState::UpToDate => {}
                SyncState::Syncing {
                    uploads: u,
                    queued: q,
                } => {
                    syncing = true;
                    uploads += u;
                    queued += q;
                }
                SyncState::Paused { queued: q } => {
                    paused = true;
                    paused_queued += q;
                }
                SyncState::Error {
                    message,
                    failed_files,
                } => {
                    errors.push(message.as_str());
                    failed += failed_files;
                }
                SyncState::Stopped => stopped = true,
                SyncState::Offline => offline = true,
            }
        }

        if !errors.is_empty() {
            SyncState::Error {
                message: errors.join("\n"),
                failed_files: failed,
            }
        } else if syncing {
            SyncState::Syncing {
                uploads,
                queued: queued + paused_queued,
            }
        } else if paused {
            SyncState::Paused {
                queued: paused_queued,
            }
        } else if stopped {
            SyncState::Stopped
        } else if offline {
            SyncState::Offline
        } else {
            SyncState::UpToDate
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregate_empty_is_up_to_date() {
        assert_eq!(SyncState::aggregate([]), SyncState::UpToDate);
    }

    #[test]
    fn aggregate_priorities() {
        let states = [
            SyncState::UpToDate,
            SyncState::Syncing {
                uploads: 1,
                queued: 2,
            },
            SyncState::Syncing {
                uploads: 3,
                queued: 0,
            },
            SyncState::Paused { queued: 4 },
        ];
        assert_eq!(
            SyncState::aggregate(&states),
            SyncState::Syncing {
                uploads: 4,
                queued: 6
            }
        );

        let with_error = [
            SyncState::Paused { queued: 1 },
            SyncState::Error {
                message: "Token expired".into(),
                failed_files: 0,
            },
        ];
        assert!(matches!(
            SyncState::aggregate(&with_error),
            SyncState::Error { .. }
        ));
    }

    #[test]
    fn serde_shape() {
        let json = serde_json::to_string(&SyncState::Paused { queued: 3 }).unwrap();
        assert_eq!(json, r#"{"state":"paused","queued":3}"#);
        let json = serde_json::to_string(&SyncState::UpToDate).unwrap();
        assert_eq!(json, r#"{"state":"up_to_date"}"#);
    }
}
