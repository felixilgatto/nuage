use nuage_core::{AccountId, Error, Result};

/// Pauses and resumes uploads, per account (`Some`) or globally (`None`).
///
/// Mounts use a very long `WriteBack`, so rclone never uploads on its own.
/// While an account is not paused, this polls `vfs/queue` and releases new
/// items with `vfs/queue-set-expiry`; while paused it releases nothing. The
/// pause state lives in Nuage's TOML configuration and survives restarts.
#[derive(Debug, Default)]
pub struct UploadControl {}

impl UploadControl {
    pub async fn pause(&self, _account: Option<&AccountId>) -> Result<()> {
        Err(Error::not_implemented("pausing uploads"))
    }

    pub async fn resume(&self, _account: Option<&AccountId>) -> Result<()> {
        Err(Error::not_implemented("resuming uploads"))
    }
}
