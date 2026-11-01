//! Status: polls `vfs/stats` and `operations/about`, aggregates the
//! per-account states and publishes events when something changes.

use std::time::{Duration, Instant};

use nuage_core::{Account, AccountStatus, Event, Quota, StatusReport, SyncState};
use nuage_rclone::DiskCache;

use crate::Inner;
use crate::mounts::remote_fs;

/// The quota changes slowly and costs an API call to the provider.
const QUOTA_INTERVAL: Duration = Duration::from_secs(300);

struct Snapshot {
    account: Account,
    paused: bool,
    mounted: bool,
    mount_error: Option<String>,
    quota_due: bool,
    /// The folder is served offline (pinned items, saved listings).
    offline: bool,
}

/// Whether an rclone error says that the provider refused the sign-in
/// (expired or revoked token, deleted OAuth client).
pub(crate) fn is_auth_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "invalid_grant",
        "invalid_client",
        "unauthorized_client",
        "token expired",
        "expired or revoked",
        "cannot fetch token",
        "couldn't fetch token",
        "failed to refresh token",
    ]
    .iter()
    .any(|s| m.contains(s))
}

/// What the user reads when an account must be reconnected.
pub(crate) fn auth_message(account: &Account) -> String {
    format!(
        "{} needs you to sign in again (the access expired or was revoked). Reconnect it: nuage account reconnect \"{}\"",
        account.display_name, account.display_name
    )
}

/// State of one account from what the engine knows.
fn account_state(
    paused: bool,
    mounted: bool,
    errors: (Option<&str>, Option<&str>),
    stats: Option<&Result<DiskCache, String>>,
) -> SyncState {
    let (mount_error, auth_error) = errors;
    if let Some(message) = auth_error.or(mount_error) {
        return SyncState::Error {
            message: message.to_owned(),
            failed_files: 0,
        };
    }
    if !mounted {
        return SyncState::Stopped;
    }
    let cache = match stats {
        None => {
            return SyncState::Error {
                message: "rclone is not running; Nuage is restarting it.".to_owned(),
                failed_files: 0,
            };
        }
        Some(Err(e)) => {
            return SyncState::Error {
                message: format!("Cannot read the upload state: {e}"),
                failed_files: 0,
            };
        }
        Some(Ok(cache)) => cache,
    };
    let pending = cache.uploads_in_progress + cache.uploads_queued;
    if cache.errored_files > 0 {
        SyncState::Error {
            message: format!(
                "{} file(s) could not be uploaded. Nuage keeps retrying; details are in the daemon log (journalctl --user -u nuaged).",
                cache.errored_files
            ),
            failed_files: cache.errored_files,
        }
    } else if cache.out_of_space {
        SyncState::Error {
            message: "The disk holding the cache is full. Free some space or lower the cache limit (nuage config set)."
                .to_owned(),
            failed_files: 0,
        }
    } else if paused {
        SyncState::Paused { queued: pending }
    } else if pending > 0 {
        SyncState::Syncing {
            uploads: cache.uploads_in_progress,
            queued: cache.uploads_queued,
        }
    } else {
        SyncState::UpToDate
    }
}

impl Inner {
    /// Polls rclone, updates the cached report, publishes the changes and
    /// returns the new report.
    pub(crate) async fn poll_status(&self) -> StatusReport {
        let keyring_wait = self.keyring_wait();
        let snapshots: Vec<Snapshot> = {
            let st = self.state.lock().await;
            st.config
                .accounts
                .iter()
                .map(|r| {
                    let runtime = st.runtime.get(&r.account.id);
                    let mounted = runtime.is_some_and(|rt| rt.mounted.is_some());
                    let quota_capable = runtime
                        .and_then(|rt| rt.capabilities.as_ref())
                        .is_some_and(|c| c.quota);
                    let quota_fresh = runtime
                        .and_then(|rt| rt.quota_at)
                        .is_some_and(|t| t.elapsed() < QUOTA_INTERVAL);
                    Snapshot {
                        account: r.account.clone(),
                        paused: r.paused,
                        mounted,
                        mount_error: keyring_wait
                            .clone()
                            .or_else(|| runtime.and_then(|rt| rt.mount_error.clone())),
                        quota_due: mounted && quota_capable && !quota_fresh,
                        offline: runtime.is_some_and(|rt| rt.offline && rt.fs.is_some()),
                    }
                })
                .collect()
        };

        let rc = self.rc().await.ok();
        let mut polled = Vec::with_capacity(snapshots.len());
        for s in &snapshots {
            let fs = remote_fs(&s.account);
            let mut stats = None;
            let mut quota = None;
            // `Some`: the sign-in was checked (and failed with this error).
            let mut auth = None;
            if s.mounted
                && let Some(rc) = &rc
            {
                stats = Some(
                    rc.vfs_stats(&fs)
                        .await
                        .map(|v| v.disk_cache.unwrap_or_default())
                        .map_err(|e| e.to_string()),
                );
                if s.quota_due {
                    match rc.about(&fs).await {
                        Ok(a) => {
                            quota = Some(Quota {
                                total: a.total,
                                used: a.used,
                                free: a.free,
                                trashed: a.trashed,
                            });
                            auth = Some(None);
                        }
                        Err(e) if is_auth_error(&e.to_string()) => {
                            tracing::warn!(%fs, error = %e, "the provider refused the sign-in");
                            auth = Some(Some(auth_message(&s.account)));
                        }
                        Err(e) => tracing::debug!(%fs, error = %e, "operations/about failed"),
                    }
                }
            }
            polled.push((stats, quota, auth));
        }

        let mut st = self.state.lock().await;
        let mut accounts = Vec::with_capacity(snapshots.len());
        let limit = st.config.settings.offline_warning_bytes;
        for (s, (stats, quota, auth)) in snapshots.iter().zip(polled) {
            // The account may have been removed while polling.
            if !st
                .config
                .accounts
                .iter()
                .any(|r| r.account.id == s.account.id)
            {
                continue;
            }
            let runtime = st.runtime(&s.account.id);
            if let Some(q) = quota {
                runtime.quota = Some(q);
                runtime.quota_at = Some(Instant::now());
            }
            if let Some(auth) = auth {
                if auth.is_some() && runtime.auth_error.is_none() {
                    self.emit(Event::AuthRequired {
                        account: s.account.id.clone(),
                    });
                }
                runtime.auth_error = auth;
            }
            if let Some(Ok(cache)) = &stats {
                let before = runtime.disk_cache.as_ref().map_or(0, |c| c.errored_files);
                if cache.errored_files > before {
                    self.emit(Event::UploadFailed {
                        account: s.account.id.clone(),
                        path: String::new(),
                        message: format!(
                            "{} file(s) of {} could not be uploaded. Nuage keeps retrying.",
                            cache.errored_files, s.account.display_name
                        ),
                    });
                }
                runtime.disk_cache = Some(cache.clone());
            }
            let state = if s.offline {
                SyncState::Offline
            } else {
                account_state(
                    s.paused,
                    s.mounted,
                    (s.mount_error.as_deref(), runtime.auth_error.as_deref()),
                    stats.as_ref(),
                )
            };
            let (offline_used, pins_pending, pins_conflicts) = self.pins.summary(&s.account.id);
            if self.pins.should_warn(&s.account.id, offline_used, limit) {
                self.emit(Event::OfflineSpaceWarning {
                    account: s.account.id.clone(),
                    used: offline_used,
                    limit: limit.unwrap_or_default(),
                });
            }
            accounts.push(AccountStatus {
                account: s.account.clone(),
                state,
                // Offline, the folder is there (served from the pins).
                mounted: s.mounted || s.offline,
                quota: runtime.quota,
                cache_used: runtime.disk_cache.as_ref().map_or(0, |c| c.bytes_used),
                offline_used,
                pins_pending,
                pins_conflicts,
            });
        }
        let report = StatusReport {
            state: SyncState::aggregate(accounts.iter().map(|a| &a.state)),
            cache_used: accounts.iter().map(|a| a.cache_used).sum(),
            accounts,
            config_encrypted: self
                .config_encrypted
                .load(std::sync::atomic::Ordering::Relaxed),
        };

        let out_of_space: Vec<_> = st
            .runtime
            .iter()
            .filter(|(_, rt)| rt.disk_cache.as_ref().is_some_and(|c| c.out_of_space))
            .map(|(id, _)| id.clone())
            .collect();
        if st.last_report.as_ref() != Some(&report) {
            let previous = st.last_report.take();
            for a in &report.accounts {
                let before = previous
                    .as_ref()
                    .and_then(|p| p.accounts.iter().find(|b| b.account.id == a.account.id));
                if before.map(|b| &b.state) != Some(&a.state) {
                    self.emit(Event::AccountStateChanged {
                        account: a.account.id.clone(),
                        state: a.state.clone(),
                    });
                    if out_of_space.contains(&a.account.id) {
                        self.emit(Event::CacheFull {
                            account: a.account.id.clone(),
                        });
                    }
                }
                if let Some(q) = a.quota
                    && before.and_then(|b| b.quota) != Some(q)
                {
                    self.emit(Event::QuotaUpdated {
                        account: a.account.id.clone(),
                        quota: q,
                    });
                }
            }
            self.emit(Event::StatusChanged {
                status: report.clone(),
            });
            st.last_report = Some(report.clone());
        }
        report
    }

    /// Uploads pending on accounts that are not paused.
    pub(crate) async fn pending_uploads(&self) -> u32 {
        let report = self.poll_status().await;
        report
            .accounts
            .iter()
            .map(|a| match a.state {
                SyncState::Syncing { uploads, queued } => uploads + queued,
                _ => 0,
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache(in_progress: u32, queued: u32) -> DiskCache {
        DiskCache {
            uploads_in_progress: in_progress,
            uploads_queued: queued,
            ..DiskCache::default()
        }
    }

    #[test]
    fn auth_errors() {
        assert!(is_auth_error(
            "couldn't fetch token: invalid_grant: maybe token expired? - try refreshing with \"rclone config reconnect gdrive:\""
        ));
        assert!(is_auth_error(
            "oauth2: \"invalid_client\" \"The OAuth client was deleted.\""
        ));
        assert!(!is_auth_error(
            "dial tcp: lookup www.googleapis.com: no such host"
        ));
        assert!(matches!(
            account_state(false, true, (Some("mount"), Some("sign in")), None),
            SyncState::Error { message, .. } if message == "sign in"
        ));
    }

    #[test]
    fn states() {
        assert_eq!(
            account_state(false, false, (None, None), None),
            SyncState::Stopped
        );
        assert!(matches!(
            account_state(false, false, (Some("boom"), None), None),
            SyncState::Error { .. }
        ));
        assert!(matches!(
            account_state(false, true, (None, None), None),
            SyncState::Error { .. }
        ));
        assert_eq!(
            account_state(false, true, (None, None), Some(&Ok(cache(0, 0)))),
            SyncState::UpToDate
        );
        assert_eq!(
            account_state(false, true, (None, None), Some(&Ok(cache(1, 2)))),
            SyncState::Syncing {
                uploads: 1,
                queued: 2
            }
        );
        assert_eq!(
            account_state(true, true, (None, None), Some(&Ok(cache(0, 3)))),
            SyncState::Paused { queued: 3 }
        );
        let failed = DiskCache {
            errored_files: 2,
            ..cache(0, 0)
        };
        assert!(matches!(
            account_state(false, true, (None, None), Some(&Ok(failed))),
            SyncState::Error {
                failed_files: 2,
                ..
            }
        ));
    }
}
