use nuage_core::{AccountId, Error, Exclusion, ExclusionRule, Result};

/// Per-account exclusion rules, turned into rclone filters for the mount
/// (`_filter` of `mount/mount`). Changing a rule remounts the account.
#[derive(Debug, Default)]
pub struct ExclusionManager {}

impl ExclusionManager {
    pub async fn add(&self, _account: &AccountId, _rule: ExclusionRule) -> Result<()> {
        Err(Error::not_implemented("adding an exclusion"))
    }

    pub async fn remove(&self, _account: &AccountId, _rule: &ExclusionRule) -> Result<()> {
        Err(Error::not_implemented("removing an exclusion"))
    }

    pub async fn list(&self, _account: Option<&AccountId>) -> Result<Vec<Exclusion>> {
        Err(Error::not_implemented("listing exclusions"))
    }
}
