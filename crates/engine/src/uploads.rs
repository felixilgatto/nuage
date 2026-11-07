//! Pause and resume of uploads.
//!
//! Every mount uses a very long `WriteBack` ([`HOLD_WRITE_BACK`]), so rclone
//! never uploads on its own. For each mounted account that is not paused,
//! the engine reads `vfs/queue` and releases the items held by that delay
//! with `vfs/queue-set-expiry`. While an account is paused, nothing is
//! released: writes pile up in the cache and leave on resume. The pause
//! state lives in `config.toml` and survives restarts.
//!
//! The queue is read every second while something may be in it ([`Watch`]):
//! after `nuage-fs` saw a file written and closed, after a mount or a
//! resume, and as long as the queue is not empty. Otherwise it is read
//! every [`IDLE_INTERVAL`], in case something was queued another way.
//!
//! [`HOLD_WRITE_BACK`]: crate::mounts::HOLD_WRITE_BACK

use std::time::{Duration, Instant};

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

/// How long the engine watches closely after something may have been
/// queued. rclone queues a file shortly after it is closed.
const BUSY_WINDOW: Duration = Duration::from_secs(10);
/// How often the upload queue and the status are read when nothing is
/// going on.
pub(crate) const IDLE_INTERVAL: Duration = Duration::from_secs(20);

/// Whether something may be going on with uploads.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    /// The value of `nuage-fs`'s write counter last seen.
    writes_seen: u64,
    busy_until: Option<Instant>,
    last_idle_check: Option<Instant>,
}

impl Watch {
    /// Something may be queued: watch closely for a while.
    pub(crate) fn busy(&mut self, now: Instant) {
        self.busy_until = Some(now + BUSY_WINDOW);
    }

    /// Whether the queue and status should be read closely now, from the
    /// current value of the write counter.
    pub(crate) fn is_busy(&mut self, writes: u64, now: Instant) -> bool {
        if writes != self.writes_seen {
            self.writes_seen = writes;
            self.busy(now);
        }
        self.busy_until.is_some_and(|t| now < t)
    }

    /// Whether the slow check is due (and then counts it as done).
    pub(crate) fn idle_check_due(&mut self, now: Instant) -> bool {
        let due = self
            .last_idle_check
            .is_none_or(|t| now.duration_since(t) >= IDLE_INTERVAL);
        if due {
            self.last_idle_check = Some(now);
        }
        due
    }
}

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
            self.watch_uploads();
            self.release_uploads().await;
        }
        self.poll_status().await;
        Ok(())
    }

    /// Something may be queued: read the queue every second for a while.
    pub(crate) fn watch_uploads(&self) {
        self.uploads
            .lock()
            .expect("upload watch")
            .busy(Instant::now());
    }

    /// Whether the queue and status should be read closely now.
    pub(crate) fn uploads_busy(&self) -> bool {
        let writes = self.writes.load(std::sync::atomic::Ordering::Relaxed);
        self.uploads
            .lock()
            .expect("upload watch")
            .is_busy(writes, Instant::now())
    }

    /// Whether the slow check of the queue and status is due.
    pub(crate) fn idle_check_due(&self) -> bool {
        self.uploads
            .lock()
            .expect("upload watch")
            .idle_check_due(Instant::now())
    }

    /// Releases the held uploads of mounted accounts that are not paused.
    /// While a queue is not empty, keeps watching closely.
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
            if !queue.is_empty() {
                self.watch_uploads();
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_closely_after_writes_then_slowly() {
        let start = Instant::now();
        let mut w = Watch::default();
        assert!(!w.is_busy(0, start));
        // A file written and closed.
        assert!(w.is_busy(1, start));
        assert!(w.is_busy(1, start + BUSY_WINDOW - Duration::from_secs(1)));
        assert!(!w.is_busy(1, start + BUSY_WINDOW));
        // Something queued (a mount, a resume, a non-empty queue).
        w.busy(start + BUSY_WINDOW);
        assert!(w.is_busy(1, start + BUSY_WINDOW + Duration::from_secs(1)));
    }

    #[test]
    fn idle_check_every_interval() {
        let start = Instant::now();
        let mut w = Watch::default();
        assert!(w.idle_check_due(start));
        assert!(!w.idle_check_due(start + Duration::from_secs(1)));
        assert!(w.idle_check_due(start + IDLE_INTERVAL));
        assert!(!w.idle_check_due(start + IDLE_INTERVAL + Duration::from_secs(1)));
    }
}
