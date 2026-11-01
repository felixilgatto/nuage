//! Pause and resume of uploads.
//!
//! Every mount uses a very long `WriteBack` ([`HOLD_WRITE_BACK`]), so rclone
//! never uploads on its own. Every second, for each mounted account that is
//! not paused, the engine reads `vfs/queue` and releases the items held by
//! that delay with `vfs/queue-set-expiry`. While an account is paused,
//! nothing is released: writes pile up in the cache and leave on resume.
//! The pause state lives in `config.toml` and survives restarts.
//!
//! [`HOLD_WRITE_BACK`]: crate::mounts::HOLD_WRITE_BACK

use nuage_core::{AccountId, Result};

use crate::Inner;
use crate::mounts::remote_fs;

/// An item is released once it has not been rewritten for this long, like
/// rclone's default 5 s write-back but shorter.
const RELEASE_AFTER_SECS: f64 = 2.0;
/// Items queued with a delay above this were held by Nuage. Shorter delays
/// are rclone's own retry back-off, which is left alone.
const HELD_DELAY_SECS: f64 = 3600.0;
/// `expiry` that makes rclone upload an item as soon as possible.
const RELEASE: f64 = -1e9;
/// `expiry` that holds an item for good.
const HOLD: f64 = 1e9;

impl Inner {
    /// Pauses or resumes uploads of one account, or of all of them.
    pub(crate) async fn set_paused(&self, account: Option<&AccountId>, paused: bool) -> Result<()> {
        let targets: Vec<String> = {
            let mut st = self.state.lock().await;
            let indices: Vec<usize> = match account {
                Some(id) => vec![st.resolve(id)?],
                None => (0..st.config.accounts.len()).collect(),
            };
            for &i in &indices {
                st.config.accounts[i].paused = paused;
            }
            self.save(&st)?;
            indices
                .into_iter()
                .filter(|&i| st.is_mounted(&st.record(i).account.id))
                .map(|i| remote_fs(&st.record(i).account))
                .collect()
        };
        if paused {
            // Items released but not started yet are held again.
            if let Ok(rc) = self.rc().await {
                for fs in targets {
                    let Ok(queue) = rc.vfs_queue(&fs).await else {
                        continue;
                    };
                    for item in queue.iter().filter(|i| !i.uploading && i.expiry <= 0.0) {
                        let _ = rc.vfs_queue_set_expiry(&fs, item.id, HOLD).await;
                    }
                }
            }
        } else {
            self.release_uploads().await;
        }
        self.poll_status().await;
        Ok(())
    }

    /// Releases the held uploads of mounted accounts that are not paused.
    pub(crate) async fn release_uploads(&self) {
        let targets: Vec<String> = {
            let st = self.state.lock().await;
            st.config
                .accounts
                .iter()
                .filter(|r| !r.paused && st.is_mounted(&r.account.id))
                .map(|r| remote_fs(&r.account))
                .collect()
        };
        if targets.is_empty() {
            return;
        }
        let Ok(rc) = self.rc().await else {
            return;
        };
        for fs in targets {
            let queue = match rc.vfs_queue(&fs).await {
                Ok(q) => q,
                Err(e) => {
                    tracing::debug!(%fs, error = %e, "vfs/queue failed");
                    continue;
                }
            };
            for item in queue {
                let held = item.delay > HELD_DELAY_SECS && item.expiry > 0.0;
                if !item.uploading && held && item.age_secs() >= RELEASE_AFTER_SECS {
                    tracing::debug!(%fs, name = %item.name, "releasing upload");
                    if let Err(e) = rc.vfs_queue_set_expiry(&fs, item.id, RELEASE).await {
                        tracing::debug!(%fs, error = %e, "vfs/queue-set-expiry failed");
                    }
                }
            }
        }
    }
}
