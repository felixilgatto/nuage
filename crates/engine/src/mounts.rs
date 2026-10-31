use std::path::PathBuf;

use nuage_core::{Account, AccountId, Error, Result};

/// Where an account is mounted.
///
/// `nuage-fs` is mounted at the user's folder, on top of a hidden rclone
/// mount (`data/mounts/<account>`). Until `nuage-fs` exists, rclone mounts
/// directly at the user's folder ([`MountPlan::direct`]). Keeping both paths
/// apart means neither the account configuration nor the interfaces change
/// when `nuage-fs` arrives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    pub account: AccountId,
    /// Folder chosen by the user (`Account::mount_point`).
    pub user_mount_point: PathBuf,
    /// Where rclone itself mounts.
    pub rclone_mount_point: PathBuf,
}

impl MountPlan {
    /// Interim plan without `nuage-fs`: rclone mounts at the user's folder.
    pub fn direct(account: &Account) -> Self {
        Self {
            account: account.id.clone(),
            user_mount_point: account.mount_point.clone(),
            rclone_mount_point: account.mount_point.clone(),
        }
    }
}

/// `mount/mount`, `mount/unmount`, remount at startup, orphan cleanup.
#[derive(Debug, Default)]
pub struct MountManager {
    mounted: Vec<MountPlan>,
}

impl MountManager {
    pub fn mounted(&self) -> &[MountPlan] {
        &self.mounted
    }

    pub async fn mount(&self, _account: &AccountId) -> Result<()> {
        Err(Error::not_implemented("mounting an account"))
    }

    pub async fn unmount(&self, _account: &AccountId) -> Result<()> {
        Err(Error::not_implemented("unmounting an account"))
    }

    /// Mounts every account at daemon startup.
    pub async fn remount_all(&self, _accounts: &[Account]) -> Result<()> {
        Err(Error::not_implemented("remounting accounts"))
    }

    pub async fn unmount_all(&self) -> Result<()> {
        Err(Error::not_implemented("unmounting all accounts"))
    }

    /// Removes mounts left behind by a crashed daemon.
    pub async fn cleanup_orphans(&self) -> Result<()> {
        Err(Error::not_implemented("cleaning up orphan mounts"))
    }
}
