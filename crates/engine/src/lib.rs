//! The Nuage engine, run inside `nuaged`.
//!
//! ```text
//! Engine
//!  ├─ RcdSupervisor     (nuage-rclone) launches and watches `rclone rcd`
//!  ├─ accounts          create, rename, remove accounts
//!  ├─ mounts            mount/unmount, remount at startup, orphan cleanup
//!  ├─ uploads           pause / resume uploads, per account or global
//!  ├─ status            polls vfs/stats and operations/about, publishes events
//!  ├─ settings          `nuage config get/set`
//!  ├─ ExclusionManager  per-account exclusion rules → rclone filters (milestone 7)
//!  ├─ FileService       list, mkdir, move, copy, delete, upload, download, link (milestone 9)
//!  └─ PinManager        offline pinning (v1.1)
//! ```
//!
//! The engine owns all the daemon state. It is shared between the IPC
//! connections and a background task that ticks every second: it restarts
//! rclone if it died, releases uploads of accounts that are not paused, and
//! polls the status.

mod accounts;
mod config;
mod exclusions;
mod files;
mod mounts;
mod paths;
mod pins;
mod settings;
mod status;
mod units;
mod uploads;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, broadcast};
use tokio::task::JoinHandle;

pub use config::QuitBehavior;
pub use exclusions::ExclusionManager;
pub use files::FileService;
pub use mounts::MountPlan;
pub use paths::Paths;
pub use pins::PinManager;
pub use units::{format_duration, format_size, parse_duration, parse_size};

pub use nuage_providers::ProviderRegistry;
pub use nuage_rclone::{RcdConfig, RcdSupervisor};

use nuage_core::{
    AccountId, Capabilities, Command, Error, Event, Quota, Reply, Result, StatusReport,
};
use nuage_rclone::{DiskCache, RcClient};

use config::{AccountRecord, AppConfig};

/// Minimum delay between two restarts of a crashed rcd.
const RESTART_BACKOFF: Duration = Duration::from_secs(5);

/// How the engine is set up by the daemon.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub paths: Paths,
    /// The rclone binary.
    pub rclone_path: PathBuf,
}

/// What the daemon does after `nuage daemon stop`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPlan {
    Now,
    /// Wait until the uploads of accounts that are not paused are done.
    AfterUploads,
}

/// Owns all the daemon state: accounts, mounts, exclusions, pins.
#[derive(Debug)]
pub struct Engine {
    inner: Arc<Inner>,
}

#[derive(Debug)]
pub(crate) struct Inner {
    registry: ProviderRegistry,
    paths: Paths,
    events: broadcast::Sender<Event>,
    state: Mutex<State>,
    rcd: Mutex<RcdState>,
    background: std::sync::Mutex<Option<JoinHandle<()>>>,
    exclusions: ExclusionManager,
    files: FileService,
    pins: PinManager,
}

#[derive(Debug)]
struct RcdState {
    supervisor: RcdSupervisor,
    client: Option<RcClient>,
    last_start: Option<Instant>,
}

#[derive(Debug, Default)]
pub(crate) struct State {
    config: AppConfig,
    runtime: BTreeMap<AccountId, Runtime>,
    last_report: Option<StatusReport>,
}

/// What the engine knows about an account while it runs.
#[derive(Debug, Default)]
pub(crate) struct Runtime {
    /// Both layers are mounted.
    mounted: Option<MountPlan>,
    /// `nuage-fs` at the user's folder. It outlives the hidden rclone mount
    /// when rclone is restarted.
    fs: Option<nuage_fs::MountedFs>,
    /// Why the last mount attempt failed.
    mount_error: Option<String>,
    capabilities: Option<Capabilities>,
    quota: Option<Quota>,
    quota_at: Option<Instant>,
    disk_cache: Option<DiskCache>,
}

impl State {
    /// Finds an account by id, then by name (exact, then ignoring case).
    pub(crate) fn resolve(&self, id: &AccountId) -> Result<usize> {
        let accounts = &self.config.accounts;
        if let Some(i) = accounts.iter().position(|r| &r.account.id == id) {
            return Ok(i);
        }
        if let Some(i) = accounts
            .iter()
            .position(|r| r.account.display_name == id.as_str())
        {
            return Ok(i);
        }
        let folded: Vec<usize> = accounts
            .iter()
            .enumerate()
            .filter(|(_, r)| r.account.display_name.eq_ignore_ascii_case(id.as_str()))
            .map(|(i, _)| i)
            .collect();
        if let [i] = folded[..] {
            return Ok(i);
        }
        let names: Vec<&str> = accounts
            .iter()
            .map(|r| r.account.display_name.as_str())
            .collect();
        Err(Error::NotFound(if names.is_empty() {
            format!(
                "No account named \"{id}\". There is no account yet: add one with `nuage account add`."
            )
        } else {
            format!("No account named \"{id}\". Accounts: {}.", names.join(", "))
        }))
    }

    pub(crate) fn record(&self, i: usize) -> &AccountRecord {
        &self.config.accounts[i]
    }

    pub(crate) fn runtime(&mut self, id: &AccountId) -> &mut Runtime {
        self.runtime.entry(id.clone()).or_default()
    }

    pub(crate) fn is_mounted(&self, id: &AccountId) -> bool {
        self.runtime.get(id).is_some_and(|r| r.mounted.is_some())
    }
}

impl Engine {
    /// Creates an engine and reads its configuration. rclone is not started.
    pub fn new(registry: ProviderRegistry, config: EngineConfig) -> Result<Self> {
        let app_config = AppConfig::load(&config.paths.app_config())?;
        let supervisor = RcdSupervisor::new(RcdConfig {
            rclone_path: config.rclone_path,
            config_path: config.paths.rclone_config.clone(),
            cache_dir: config.paths.cache_dir.clone(),
            pid_file: Some(config.paths.rcd_pid_file()),
        });
        let (events, _) = broadcast::channel(256);
        Ok(Self {
            inner: Arc::new(Inner {
                registry,
                paths: config.paths,
                events,
                state: Mutex::new(State {
                    config: app_config,
                    ..State::default()
                }),
                rcd: Mutex::new(RcdState {
                    supervisor,
                    client: None,
                    last_start: None,
                }),
                background: std::sync::Mutex::new(None),
                exclusions: ExclusionManager::default(),
                files: FileService::default(),
                pins: PinManager::default(),
            }),
        })
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.inner.registry
    }

    pub fn paths(&self) -> &Paths {
        &self.inner.paths
    }

    /// Sender of the events published by the engine, for the IPC server.
    pub fn events(&self) -> broadcast::Sender<Event> {
        self.inner.events.clone()
    }

    /// Version of the running rclone.
    pub async fn rclone_version(&self) -> Option<String> {
        let rcd = self.inner.rcd.lock().await;
        rcd.supervisor.version().map(str::to_owned)
    }

    /// Starts rclone, cleans up orphan mounts, remounts the accounts and
    /// starts the background task.
    pub async fn start(&self) -> Result<()> {
        self.inner.start_rcd().await?;
        self.inner.remount_all().await;
        let inner = Arc::clone(&self.inner);
        let handle = tokio::spawn(async move { inner.background().await });
        if let Some(old) = self
            .inner
            .background
            .lock()
            .expect("background lock")
            .replace(handle)
        {
            old.abort();
        }
        self.inner.poll_status().await;
        Ok(())
    }

    /// Stops the background task, unmounts every account, then stops rclone.
    pub async fn shutdown(&self) -> Result<()> {
        if let Some(handle) = self
            .inner
            .background
            .lock()
            .expect("background lock")
            .take()
        {
            handle.abort();
        }
        self.inner.unmount_all().await;
        let mut rcd = self.inner.rcd.lock().await;
        rcd.client = None;
        rcd.supervisor.stop().await
    }

    /// Decides how to honour `nuage daemon stop`, following the "quit with
    /// pending uploads" setting.
    pub async fn stop_plan(&self, now: bool) -> Result<StopPlan> {
        if now {
            return Ok(StopPlan::Now);
        }
        let pending = self.inner.pending_uploads().await;
        if pending == 0 {
            return Ok(StopPlan::Now);
        }
        let behavior = self
            .inner
            .state
            .lock()
            .await
            .config
            .settings
            .quit_with_pending;
        match behavior {
            QuitBehavior::FinishInBackground => Ok(StopPlan::AfterUploads),
            QuitBehavior::QuitNow => Ok(StopPlan::Now),
            QuitBehavior::Ask => Err(Error::Busy(format!(
                "{pending} file(s) are still uploading. Wait for them, or stop now with `nuage daemon stop --now` (they upload at the next start)."
            ))),
        }
    }

    /// Returns once no account that is not paused has pending uploads.
    pub async fn wait_for_uploads(&self) {
        loop {
            let pending = self.inner.pending_uploads().await;
            if pending == 0 {
                return;
            }
            tracing::info!(pending, "waiting for uploads before stopping");
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// Executes a client command.
    pub async fn handle(&self, command: Command) -> Result<Reply> {
        let inner = &self.inner;
        match command {
            Command::Status => Ok(Reply::Status(inner.poll_status().await)),
            Command::ListProviders => Ok(Reply::Providers(inner.registry.infos())),
            Command::AccountList => Ok(Reply::Accounts(inner.list_accounts().await)),
            Command::AccountAdd(args) => inner.add_account(args).await.map(Reply::Account),
            Command::AccountRename { account, name } => inner
                .rename_account(&account, &name)
                .await
                .map(Reply::Account),
            Command::AccountReconnect { account } => inner
                .reconnect_account(&account)
                .await
                .map(|()| Reply::Done),
            Command::AccountRemove { account, force } => inner
                .remove_account(&account, force)
                .await
                .map(|()| Reply::Done),
            Command::Mount { account } => inner.mount_command(&account).await.map(|()| Reply::Done),
            Command::Unmount { account } => {
                inner.unmount_command(&account).await.map(|()| Reply::Done)
            }
            Command::Pause { account } => inner
                .set_paused(account.as_ref(), true)
                .await
                .map(|()| Reply::Done),
            Command::Resume { account } => inner
                .set_paused(account.as_ref(), false)
                .await
                .map(|()| Reply::Done),
            Command::Refresh { account } => {
                inner.refresh(account.as_ref()).await.map(|()| Reply::Done)
            }
            Command::ConfigGet { key } => inner.config_get(key.as_deref()).await.map(Reply::Config),
            Command::ConfigSet { key, value } => {
                inner.config_set(&key, &value).await.map(Reply::Config)
            }
            Command::List { path } => inner.files.list(&path).await.map(Reply::Entries),
            Command::Mkdir { path } => inner.files.mkdir(&path).await.map(|()| Reply::Done),
            Command::Move { from, to } => {
                inner.files.move_to(&from, &to).await.map(|()| Reply::Done)
            }
            Command::Copy { from, to } => {
                inner.files.copy_to(&from, &to).await.map(|()| Reply::Done)
            }
            Command::Delete { path, recursive } => inner
                .files
                .delete(&path, recursive)
                .await
                .map(|()| Reply::Done),
            Command::Upload { local, to } => {
                inner.files.upload(&local, &to).await.map(|()| Reply::Done)
            }
            Command::Download { from, local } => inner
                .files
                .download(&from, &local)
                .await
                .map(|()| Reply::Done),
            Command::PublicLink { path } => inner.files.public_link(&path).await.map(Reply::Link),
            Command::ExcludeAdd { account, rule } => inner
                .exclusions
                .add(&account, rule)
                .await
                .map(|()| Reply::Done),
            Command::ExcludeRemove { account, rule } => inner
                .exclusions
                .remove(&account, &rule)
                .await
                .map(|()| Reply::Done),
            Command::ExcludeList { account } => inner
                .exclusions
                .list(account.as_ref())
                .await
                .map(Reply::Exclusions),
            Command::PinAdd { path, kind } => {
                inner.pins.add(&path, kind).await.map(|()| Reply::Done)
            }
            Command::PinRemove { path } => inner.pins.remove(&path).await.map(|()| Reply::Done),
            Command::PinList { account } => {
                inner.pins.list(account.as_ref()).await.map(Reply::Pins)
            }
            // Daemon-level commands are answered by `nuaged` itself.
            Command::DaemonInfo | Command::DaemonStop { .. } => Err(Error::Internal(
                "daemon command routed to the engine".to_owned(),
            )),
        }
    }
}

impl Inner {
    pub(crate) fn emit(&self, event: Event) {
        // No subscriber is not an error.
        let _ = self.events.send(event);
    }

    /// Client of the running rcd.
    pub(crate) async fn rc(&self) -> Result<RcClient> {
        self.rcd.lock().await.client.clone().ok_or_else(|| {
            Error::Rclone(
                "rclone is not running. Nuage restarts it automatically; if this persists, check the daemon log (journalctl --user -u nuaged)."
                    .to_owned(),
            )
        })
    }

    async fn start_rcd(&self) -> Result<()> {
        let mut rcd = self.rcd.lock().await;
        rcd.last_start = Some(Instant::now());
        let client = rcd.supervisor.start().await?;
        rcd.client = Some(client);
        Ok(())
    }

    /// Restarts rcd if it died, then remounts the accounts.
    async fn check_rcd(&self) {
        let mut rcd = self.rcd.lock().await;
        if let Some(status) = rcd.supervisor.exited() {
            tracing::error!(%status, "rclone rcd exited unexpectedly");
            rcd.client = None;
            drop(rcd);
            let mut st = self.state.lock().await;
            self.hidden_mounts_lost(&mut st);
            drop(st);
            rcd = self.rcd.lock().await;
        }
        if rcd.client.is_some()
            || rcd
                .last_start
                .is_some_and(|t| t.elapsed() < RESTART_BACKOFF)
        {
            return;
        }
        rcd.last_start = Some(Instant::now());
        match rcd.supervisor.start().await {
            Ok(client) => {
                rcd.client = Some(client);
                drop(rcd);
                tracing::info!("rclone rcd restarted");
                self.remount_all().await;
            }
            Err(e) => tracing::error!(error = %e, "cannot restart rclone rcd"),
        }
    }

    async fn background(self: Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut n: u64 = 0;
        loop {
            tick.tick().await;
            n += 1;
            self.check_rcd().await;
            self.release_uploads().await;
            if n % 2 == 0 {
                self.poll_status().await;
            }
        }
    }

    /// `vfs/forget` + `vfs/refresh` of one account or of every mounted one.
    async fn refresh(&self, account: Option<&AccountId>) -> Result<()> {
        let targets: Vec<(String, String)> = {
            let st = self.state.lock().await;
            let indices: Vec<usize> = match account {
                Some(id) => {
                    let i = st.resolve(id)?;
                    if !st.is_mounted(&st.record(i).account.id) {
                        return Err(Error::InvalidArgument(format!(
                            "\"{}\" is not mounted. Mount it with `nuage mount`.",
                            st.record(i).account.display_name
                        )));
                    }
                    vec![i]
                }
                None => (0..st.config.accounts.len())
                    .filter(|&i| st.is_mounted(&st.record(i).account.id))
                    .collect(),
            };
            indices
                .into_iter()
                .map(|i| {
                    let a = &st.record(i).account;
                    (mounts::remote_fs(a), a.display_name.clone())
                })
                .collect()
        };
        let rc = self.rc().await?;
        for (fs, name) in targets {
            rc.vfs_refresh(&fs)
                .await
                .map_err(|e| Error::Rclone(format!("cannot refresh \"{name}\": {e}")))?;
        }
        Ok(())
    }

    pub(crate) fn save(&self, st: &State) -> Result<()> {
        st.config.save(&self.paths.app_config())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(root: &std::path::Path) -> Engine {
        Engine::new(
            ProviderRegistry::with_enabled_providers(),
            EngineConfig {
                paths: Paths::under(root),
                rclone_path: root.join("no-rclone"),
            },
        )
        .unwrap()
    }

    #[tokio::test]
    async fn status_of_an_empty_engine() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let reply = engine.handle(Command::Status).await.unwrap();
        assert_eq!(reply, Reply::Status(StatusReport::empty()));
    }

    #[tokio::test]
    async fn providers_are_listed() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let Reply::Providers(infos) = engine.handle(Command::ListProviders).await.unwrap() else {
            panic!("unexpected reply");
        };
        assert!(infos.iter().any(|p| p.id == "local"));
    }

    #[tokio::test]
    async fn unknown_account() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        let err = engine
            .handle(Command::Mount {
                account: AccountId::new("Drive"),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    }

    #[tokio::test]
    async fn missing_rclone_fails_to_start() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(dir.path());
        assert!(matches!(engine.start().await, Err(Error::Rclone(_))));
    }
}
