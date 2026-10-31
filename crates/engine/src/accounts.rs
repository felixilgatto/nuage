use nuage_core::{Account, AccountAddArgs, AccountId, Error, Result};
use nuage_providers::ProviderRegistry;

/// Creates, renames, removes and reconnects accounts.
///
/// Accounts are persisted in the application's TOML configuration and the
/// matching remotes in `rclone.conf`.
#[derive(Debug, Default)]
pub struct AccountManager {
    accounts: Vec<Account>,
}

impl AccountManager {
    pub fn list(&self) -> &[Account] {
        &self.accounts
    }

    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.accounts.iter().find(|a| &a.id == id)
    }

    /// Adds an account following the provider's `AuthSpec`.
    pub async fn add(&self, registry: &ProviderRegistry, args: AccountAddArgs) -> Result<Account> {
        if registry.get(&args.provider).is_none() {
            let known: Vec<_> = registry.iter().map(|p| p.id()).collect();
            return Err(Error::InvalidArgument(format!(
                "Unknown provider \"{}\". Available providers: {}.",
                args.provider,
                known.join(", ")
            )));
        }
        Err(Error::not_implemented("adding an account"))
    }

    pub async fn rename(&self, _account: &AccountId, _name: &str) -> Result<Account> {
        Err(Error::not_implemented("renaming an account"))
    }

    pub async fn reconnect(&self, _account: &AccountId) -> Result<()> {
        Err(Error::not_implemented("reconnecting an account"))
    }

    /// Unmounts, then deletes the remote, the cache, the pins and the mount
    /// folder if empty. Nothing is deleted in the cloud.
    pub async fn remove(&self, _account: &AccountId, _force: bool) -> Result<()> {
        Err(Error::not_implemented("removing an account"))
    }
}
