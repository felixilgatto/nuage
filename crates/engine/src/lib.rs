//! The Nuage engine, run inside `nuaged`.
//!
//! ```text
//! Engine
//!  ├─ RcdSupervisor     (nuage-rclone) launches and watches `rclone rcd`
//!  ├─ AccountManager    create, rename, remove, reconnect accounts
//!  ├─ MountManager      mount/unmount, remount at startup, orphan cleanup
//!  ├─ UploadControl     pause / resume uploads, per account or global
//!  ├─ ExclusionManager  per-account exclusion rules → rclone filters
//!  ├─ StatusPoller      polls vfs/stats, core/stats, operations/about
//!  ├─ FileService       list, mkdir, move, copy, delete, upload, download, link
//!  └─ PinManager        offline pinning (v1.1)
//! ```
//!
//! **Skeleton**: only `Status` and `ListProviders` are answered; every other
//! command returns [`Error::NotImplemented`]. The managers are filled in from
//! milestone 3 on.

mod accounts;
mod exclusions;
mod files;
mod mounts;
mod pins;
mod status;
mod uploads;

pub use accounts::AccountManager;
pub use exclusions::ExclusionManager;
pub use files::FileService;
pub use mounts::{MountManager, MountPlan};
pub use pins::PinManager;
pub use status::StatusPoller;
pub use uploads::UploadControl;

pub use nuage_providers::ProviderRegistry;
pub use nuage_rclone::{RcdConfig, RcdSupervisor};

use nuage_core::{Command, Error, Reply, Result};

/// Owns all the daemon state: accounts, mounts, exclusions, pins.
#[derive(Debug)]
pub struct Engine {
    registry: ProviderRegistry,
    rcd: Option<RcdSupervisor>,
    accounts: AccountManager,
    mounts: MountManager,
    uploads: UploadControl,
    exclusions: ExclusionManager,
    status: StatusPoller,
    files: FileService,
    pins: PinManager,
}

impl Engine {
    /// Creates an engine with the given providers. rclone is not started.
    pub fn new(registry: ProviderRegistry) -> Self {
        Self {
            registry,
            rcd: None,
            accounts: AccountManager::default(),
            mounts: MountManager::default(),
            uploads: UploadControl::default(),
            exclusions: ExclusionManager::default(),
            status: StatusPoller::default(),
            files: FileService::default(),
            pins: PinManager::default(),
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    /// Starts rclone, cleans up orphan mounts and remounts the accounts.
    pub async fn start(&mut self, rcd: RcdConfig) -> Result<()> {
        let mut supervisor = RcdSupervisor::new(rcd);
        supervisor.start().await?;
        self.rcd = Some(supervisor);
        self.mounts.cleanup_orphans().await?;
        self.mounts.remount_all(self.accounts.list()).await
    }

    /// Unmounts every account, then stops rclone.
    pub async fn shutdown(&mut self) -> Result<()> {
        if let Some(rcd) = self.rcd.as_mut() {
            self.mounts.unmount_all().await?;
            rcd.stop().await?;
        }
        Ok(())
    }

    /// Executes a client command.
    pub async fn handle(&self, command: Command) -> Result<Reply> {
        match command {
            Command::Status => Ok(Reply::Status(self.status.report(self.accounts.list()))),
            Command::ListProviders => Ok(Reply::Providers(self.registry.infos())),
            Command::AccountList => Ok(Reply::Accounts(self.accounts.list().to_vec())),
            Command::AccountAdd(args) => self
                .accounts
                .add(&self.registry, args)
                .await
                .map(Reply::Account),
            Command::AccountRename { account, name } => self
                .accounts
                .rename(&account, &name)
                .await
                .map(Reply::Account),
            Command::AccountReconnect { account } => self
                .accounts
                .reconnect(&account)
                .await
                .map(|()| Reply::Done),
            Command::AccountRemove { account, force } => self
                .accounts
                .remove(&account, force)
                .await
                .map(|()| Reply::Done),
            Command::Mount { account } => self.mounts.mount(&account).await.map(|()| Reply::Done),
            Command::Unmount { account } => {
                self.mounts.unmount(&account).await.map(|()| Reply::Done)
            }
            Command::Pause { account } => self
                .uploads
                .pause(account.as_ref())
                .await
                .map(|()| Reply::Done),
            Command::Resume { account } => self
                .uploads
                .resume(account.as_ref())
                .await
                .map(|()| Reply::Done),
            Command::Refresh { account } => self
                .files
                .refresh(account.as_ref())
                .await
                .map(|()| Reply::Done),
            Command::List { path } => self.files.list(&path).await.map(Reply::Entries),
            Command::Mkdir { path } => self.files.mkdir(&path).await.map(|()| Reply::Done),
            Command::Move { from, to } => {
                self.files.move_to(&from, &to).await.map(|()| Reply::Done)
            }
            Command::Copy { from, to } => {
                self.files.copy_to(&from, &to).await.map(|()| Reply::Done)
            }
            Command::Delete { path, recursive } => self
                .files
                .delete(&path, recursive)
                .await
                .map(|()| Reply::Done),
            Command::Upload { local, to } => {
                self.files.upload(&local, &to).await.map(|()| Reply::Done)
            }
            Command::Download { from, local } => self
                .files
                .download(&from, &local)
                .await
                .map(|()| Reply::Done),
            Command::PublicLink { path } => self.files.public_link(&path).await.map(Reply::Link),
            Command::ExcludeAdd { account, rule } => self
                .exclusions
                .add(&account, rule)
                .await
                .map(|()| Reply::Done),
            Command::ExcludeRemove { account, rule } => self
                .exclusions
                .remove(&account, &rule)
                .await
                .map(|()| Reply::Done),
            Command::ExcludeList { account } => self
                .exclusions
                .list(account.as_ref())
                .await
                .map(Reply::Exclusions),
            Command::PinAdd { path, kind } => {
                self.pins.add(&path, kind).await.map(|()| Reply::Done)
            }
            Command::PinRemove { path } => self.pins.remove(&path).await.map(|()| Reply::Done),
            Command::PinList { account } => self.pins.list(account.as_ref()).await.map(Reply::Pins),
            Command::ConfigGet { .. } => Err(Error::not_implemented("config get")),
            Command::ConfigSet { .. } => Err(Error::not_implemented("config set")),
            // Daemon-level commands are answered by `nuaged` itself.
            Command::DaemonInfo | Command::DaemonStop { .. } => Err(Error::Internal(
                "daemon command routed to the engine".to_owned(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::{AccountId, StatusReport};

    #[tokio::test]
    async fn status_of_an_empty_engine() {
        let engine = Engine::new(ProviderRegistry::with_enabled_providers());
        let reply = engine.handle(Command::Status).await.unwrap();
        assert_eq!(reply, Reply::Status(StatusReport::empty()));
    }

    #[tokio::test]
    async fn providers_are_listed() {
        let engine = Engine::new(ProviderRegistry::with_enabled_providers());
        let Reply::Providers(infos) = engine.handle(Command::ListProviders).await.unwrap() else {
            panic!("unexpected reply");
        };
        assert!(infos.iter().any(|p| p.id == "local"));
    }

    #[tokio::test]
    async fn stubs_are_not_implemented() {
        let engine = Engine::new(ProviderRegistry::new());
        let err = engine
            .handle(Command::Mount {
                account: AccountId::new("Drive"),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotImplemented(_)));
    }
}
