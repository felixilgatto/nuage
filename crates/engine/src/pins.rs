//! Offline pinning (v1.1): pinned items are kept in the pin store, served by
//! `nuage-fs`, and synced here through rclone.
//!
//! Every round (the check interval, after a pin is added, when the
//! connection comes back) compares, for each item under a pin, the remote
//! state (`operations/list` with hashes), the store copy and the reference
//! state saved at the last sync (see `docs/research/2026-11-01-milestone-12.md`):
//! only one side changed → it wins; both changed → a conflict, both kept.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use nuage_core::{
    Account, AccountId, Error, Event, ExclusionSet, Pin, PinKind, PinState, RemotePath, Result,
};
use nuage_rclone::{ListItem, RcClient, RcError};

use crate::Inner;
use crate::files::is_offline;
use crate::mounts::remote_fs;
use crate::pindb::{LocalSig, PinDb, PinRow, Reference, RemoteSig, SavedItem, within};

/// A file changed less than this long ago may still be being written: it
/// is uploaded at the next round.
const QUIET: Duration = Duration::from_secs(3);

/// Default of the `offline-check-interval` setting.
pub(crate) const DEFAULT_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Tests simulate the loss of the network: while the file named by this
/// variable exists, every account is offline.
const TEST_OFFLINE: &str = "NUAGE_TEST_OFFLINE";

pub(crate) fn test_offline() -> bool {
    std::env::var_os(TEST_OFFLINE).is_some_and(|p| Path::new(&p).exists())
}

/// What the interface shows of a pin.
#[derive(Debug, Clone, Copy, Default)]
struct PinStatus {
    state: PinState,
    size: u64,
    pending: u32,
}

/// The pinning part of the engine.
#[derive(Debug)]
pub(crate) struct Pins {
    pub(crate) db: PinDb,
    /// One sync round at a time.
    sync: tokio::sync::Mutex<()>,
    status: std::sync::Mutex<HashMap<(AccountId, String), PinStatus>>,
    conflicts: std::sync::Mutex<HashMap<AccountId, u32>>,
    last_check: std::sync::Mutex<Option<Instant>>,
    /// Accounts warned about the space their offline items take.
    warned: std::sync::Mutex<std::collections::HashSet<AccountId>>,
}

impl Pins {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            db: PinDb::open(path)?,
            sync: tokio::sync::Mutex::new(()),
            status: Default::default(),
            conflicts: Default::default(),
            last_check: Default::default(),
            warned: Default::default(),
        })
    }

    /// Whether to warn about `used` bytes kept offline (once per crossing
    /// of `limit`).
    pub fn should_warn(&self, account: &AccountId, used: u64, limit: Option<u64>) -> bool {
        crossed(
            &mut self.warned.lock().expect("pin warning lock"),
            account,
            used,
            limit,
        )
    }

    /// Bytes kept offline, pending changes and conflicts of an account.
    pub fn summary(&self, account: &AccountId) -> (u64, u32, u32) {
        let status = self.status.lock().expect("pin status lock");
        let (size, pending) = status
            .iter()
            .filter(|((a, _), _)| a == account)
            .fold((0, 0), |(s, p), (_, st)| (s + st.size, p + st.pending));
        let conflicts = self
            .conflicts
            .lock()
            .expect("pin conflicts lock")
            .get(account)
            .copied()
            .unwrap_or(0);
        (size, pending, conflicts)
    }
}

/// `true` the first time `used` goes over `limit`; again only after it
/// went back under.
fn crossed(
    warned: &mut std::collections::HashSet<AccountId>,
    account: &AccountId,
    used: u64,
    limit: Option<u64>,
) -> bool {
    match limit {
        Some(limit) if used > limit => warned.insert(account.clone()),
        _ => {
            warned.remove(account);
            false
        }
    }
}

/// `a/b/c` → `a/b`.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// The folders above `path`, from the root (`""`).
fn ancestors(path: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut acc = String::new();
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for part in parts.iter().take(parts.len().saturating_sub(1)) {
        acc = join(&acc, part);
        out.push(acc.clone());
    }
    out
}

/// `docs/report.odt` → `docs/report (conflict 2026-10-29 18-45).odt`.
fn conflict_name(path: &str, at: chrono::DateTime<chrono::Local>) -> String {
    let (dir, name) = path.rsplit_once('/').map_or(("", path), |(d, n)| (d, n));
    let stamp = at.format("%Y-%m-%d %H-%M");
    let name = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (conflict {stamp}).{ext}"),
        _ => format!("{name} (conflict {stamp})"),
    };
    join(dir, &name)
}

fn remote_sig(item: &ListItem) -> RemoteSig {
    RemoteSig {
        size: item.size,
        mtime: item.mod_time.clone().unwrap_or_default(),
        hash: item
            .hashes
            .iter()
            .next()
            .map(|(algo, hex)| format!("{algo}:{hex}")),
    }
}

fn local_sig(meta: &std::fs::Metadata) -> LocalSig {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i128);
    LocalSig {
        size: meta.len(),
        mtime_ns,
    }
}

#[derive(Debug, Clone)]
struct LocalItem {
    dir: bool,
    sig: LocalSig,
    modified: SystemTime,
}

#[derive(Debug, Clone)]
struct RemoteItem {
    dir: bool,
    sig: RemoteSig,
}

/// The store copies under a pin.
fn local_tree(store: &Path, pin: &PinRow, exclusions: &ExclusionSet) -> HashMap<String, LocalItem> {
    let mut out = HashMap::new();
    let item = |meta: &std::fs::Metadata| LocalItem {
        dir: meta.is_dir(),
        sig: local_sig(meta),
        modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
    };
    let Ok(meta) = std::fs::symlink_metadata(store.join(&pin.path)) else {
        return out;
    };
    if pin.kind == PinKind::File || !meta.is_dir() {
        if meta.is_file() {
            out.insert(pin.path.clone(), item(&meta));
        }
        return out;
    }
    let mut stack = vec![pin.path.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(store.join(&dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let rel = join(&dir, &name);
            if exclusions.is_excluded(&rel) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(rel.clone());
                out.insert(rel, item(&meta));
            } else if meta.is_file() {
                out.insert(rel, item(&meta));
            }
        }
    }
    out
}

/// The remote items under a pin. `Ok(None)`: the pin itself is gone.
async fn remote_tree(
    rc: &RcClient,
    fs: &str,
    pin: &PinRow,
    exclusions: &ExclusionSet,
) -> std::result::Result<Option<HashMap<String, RemoteItem>>, RcError> {
    let mut out = HashMap::new();
    if !pin.path.is_empty() {
        let Some(item) = rc.stat(fs, &pin.path).await? else {
            return Ok(None);
        };
        if !item.is_dir {
            out.insert(
                pin.path.clone(),
                RemoteItem {
                    dir: false,
                    sig: remote_sig(&item),
                },
            );
            return Ok(Some(out));
        }
    }
    for item in rc.list_recursive(fs, &pin.path).await? {
        if exclusions.is_excluded(&item.path) {
            continue;
        }
        out.insert(
            item.path.clone(),
            RemoteItem {
                dir: item.is_dir,
                sig: remote_sig(&item),
            },
        );
    }
    Ok(Some(out))
}

/// What a sync round needs about an account.
#[derive(Debug, Clone)]
struct Ctx {
    account: Account,
    fs: String,
    store: PathBuf,
    paused: bool,
    exclusions: ExclusionSet,
}

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

fn path_text(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        Error::InvalidArgument(format!("\"{}\" is not valid UTF-8.", path.display()))
    })
}

impl Inner {
    fn ctx(&self, st: &crate::State, i: usize) -> Ctx {
        let record = st.record(i);
        Ctx {
            account: record.account.clone(),
            fs: remote_fs(&record.account),
            store: self.paths.pin_store(&record.account.id),
            paused: record.paused,
            exclusions: ExclusionSet::new(&record.exclusion_rules()),
        }
    }

    /// What `nuage-fs` must know about the pins of `account`.
    pub(crate) fn pin_view(&self, account: &AccountId) -> nuage_fs::PinView {
        let store = self.paths.pin_store(account);
        let pins = self.pins.db.pins(Some(account)).unwrap_or_default();
        let refs = self.pins.db.references(account).unwrap_or_default();
        let hidden = refs
            .iter()
            .filter(|(path, r)| {
                r.remote.is_some() && std::fs::symlink_metadata(store.join(path)).is_err()
            })
            .map(|(path, _)| PathBuf::from(path))
            .collect();
        let listings = self
            .pins
            .db
            .listings(account)
            .unwrap_or_default()
            .into_iter()
            .map(|(dir, items)| {
                (
                    PathBuf::from(dir),
                    items
                        .into_iter()
                        .map(|i| nuage_fs::Listed {
                            name: i.name.into(),
                            dir: i.dir,
                            size: i.size,
                            modified: SystemTime::UNIX_EPOCH
                                + Duration::from_secs(i.mtime.max(0) as u64),
                        })
                        .collect(),
                )
            })
            .collect();
        nuage_fs::PinView {
            pins: pins.into_iter().map(|p| PathBuf::from(p.path)).collect(),
            hidden,
            listings,
        }
    }

    /// Gives `nuage-fs` the current pins of `account`.
    async fn push_view(&self, account: &AccountId) {
        let view = self.pin_view(account);
        let st = self.state.lock().await;
        if let Some(fs) = st.runtime.get(account).and_then(|r| r.fs.as_ref()) {
            fs.set_pins(view);
        }
    }

    /// Saves a listing for browsing offline (best effort).
    pub(crate) fn save_listing(&self, account: &AccountId, dir: &str, items: &[ListItem]) {
        let saved: Vec<SavedItem> = items
            .iter()
            .map(|i| SavedItem {
                name: i.name.clone(),
                dir: i.is_dir,
                size: i.size.max(0) as u64,
                mtime: i
                    .mod_time
                    .as_deref()
                    .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                    .map_or(0, |t| t.timestamp()),
            })
            .collect();
        if let Err(e) = self.pins.db.save_listing(account, dir, &saved) {
            tracing::debug!(error = %e, "cannot save a listing");
        }
    }

    // Commands

    pub(crate) async fn pin_add(&self, at: &RemotePath) -> Result<()> {
        let (ctx, path) = {
            let st = self.state.lock().await;
            let i = st.resolve(&at.account)?;
            (self.ctx(&st, i), crate::files::clean(&at.path)?)
        };
        let id = ctx.account.id.clone();
        let show = format!("{}:{path}", ctx.account.display_name);
        if !path.is_empty() && ctx.exclusions.is_excluded(&path) {
            return Err(Error::InvalidArgument(format!(
                "\"{show}\" is excluded from the sync: include it again before keeping it offline."
            )));
        }
        let existing = self.pins.db.pins(Some(&id))?;
        if let Some(p) = existing
            .iter()
            .find(|p| within(&path, &p.path) && p.kind == PinKind::FolderRecursive)
        {
            return Err(Error::AlreadyExists(format!(
                "\"{show}\" is already kept offline, with \"{}:{}\".",
                ctx.account.display_name, p.path
            )));
        }
        if existing.iter().any(|p| p.path == path) {
            return Err(Error::AlreadyExists(format!(
                "\"{show}\" is already kept offline."
            )));
        }
        if test_offline() {
            return Err(Error::Rclone(format!(
                "{} cannot be reached: keep items offline once the connection is back.",
                ctx.account.display_name
            )));
        }
        let rc = self.rc().await?;
        let kind = if path.is_empty() {
            PinKind::FolderRecursive
        } else {
            match rc.stat(&ctx.fs, &path).await {
                Ok(Some(item)) if item.is_dir => PinKind::FolderRecursive,
                Ok(Some(_)) => PinKind::File,
                Ok(None) => return Err(Error::NotFound(format!("\"{show}\" does not exist."))),
                Err(e) if is_offline(&e) => {
                    return Err(Error::Rclone(format!(
                        "{} cannot be reached: keep items offline once the connection is back.",
                        ctx.account.display_name
                    )));
                }
                Err(e) => return Err(Error::Rclone(format!("cannot read \"{show}\": {e}"))),
            }
        };
        // A folder pin covers the pins inside it.
        if kind == PinKind::FolderRecursive {
            for p in existing.iter().filter(|p| within(&p.path, &path)) {
                self.pins.db.remove_pin(&id, &p.path)?;
                self.forget_status(&id, &p.path);
            }
        }
        self.pins.db.add_pin(&id, &path, kind)?;
        self.set_status(
            &id,
            &path,
            PinStatus {
                state: PinState::Downloading,
                ..PinStatus::default()
            },
        );
        self.push_view(&id).await;
        tracing::info!(pin = %show, "pinned");
        self.sync_account(&id).await
    }

    pub(crate) async fn pin_remove(&self, at: &RemotePath, force: bool) -> Result<()> {
        let (ctx, path) = {
            let st = self.state.lock().await;
            let i = st.resolve(&at.account)?;
            (self.ctx(&st, i), crate::files::clean(&at.path)?)
        };
        let id = ctx.account.id.clone();
        let show = format!("{}:{path}", ctx.account.display_name);
        let pins = self.pins.db.pins(Some(&id))?;
        let Some(pin) = pins.into_iter().find(|p| p.path == path) else {
            return Err(Error::NotFound(format!("\"{show}\" is not kept offline.")));
        };
        let _round = self.pins.sync.lock().await;
        if !force {
            let pending = self.pin_pending(&ctx, &pin)?;
            if pending > 0 {
                return Err(Error::Busy(format!(
                    "{pending} change(s) of \"{show}\" are not uploaded yet. Wait for the connection (or resume uploads), or remove the offline copy anyway with --force (the changes are lost)."
                )));
            }
        }
        self.pins.db.remove_pin(&id, &path)?;
        self.forget_status(&id, &path);
        self.push_view(&id).await;
        let copy = ctx.store.join(&path);
        let removed = if copy.is_dir() {
            std::fs::remove_dir_all(&copy)
        } else {
            std::fs::remove_file(&copy)
        };
        if let Err(e) = removed
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %copy.display(), error = %e, "cannot remove an offline copy");
        }
        remove_empty_parents(&ctx.store, &path);
        tracing::info!(pin = %show, "unpinned");
        self.poll_status().await;
        Ok(())
    }

    pub(crate) async fn pin_list(&self, account: Option<&AccountId>) -> Result<Vec<Pin>> {
        let id = match account {
            Some(a) => {
                let st = self.state.lock().await;
                Some(st.record(st.resolve(a)?).account.id.clone())
            }
            None => None,
        };
        let rows = self.pins.db.pins(id.as_ref())?;
        let status = self.pins.status.lock().expect("pin status lock");
        Ok(rows
            .into_iter()
            .map(|r| {
                let s = status
                    .get(&(r.account.clone(), r.path.clone()))
                    .copied()
                    .unwrap_or_default();
                Pin {
                    account: r.account,
                    path: r.path,
                    kind: r.kind,
                    pinned_at: r.pinned_at,
                    state: s.state,
                    size: s.size,
                    pending: s.pending,
                }
            })
            .collect())
    }

    /// Forgets the pins of a removed account and deletes their copies.
    pub(crate) fn pins_account_removed(&self, account: &AccountId) {
        if let Err(e) = self.pins.db.remove_account(account) {
            tracing::warn!(error = %e, "cannot forget the pins of a removed account");
        }
        self.pins
            .status
            .lock()
            .expect("pin status lock")
            .retain(|(a, _), _| a != account);
        let store = self.paths.pin_store(account);
        if store.exists()
            && let Err(e) = std::fs::remove_dir_all(&store)
        {
            tracing::warn!(error = %e, "cannot delete the offline copies of a removed account");
        }
    }

    fn set_status(&self, id: &AccountId, path: &str, status: PinStatus) {
        let previous = self
            .pins
            .status
            .lock()
            .expect("pin status lock")
            .insert((id.clone(), path.to_owned()), status);
        if previous.map(|p| p.state) != Some(status.state) {
            self.emit(Event::PinStateChanged {
                account: id.clone(),
                path: path.to_owned(),
                state: status.state,
            });
        }
    }

    fn forget_status(&self, id: &AccountId, path: &str) {
        self.pins
            .status
            .lock()
            .expect("pin status lock")
            .remove(&(id.clone(), path.to_owned()));
    }

    /// Local changes under a pin that are not uploaded yet.
    fn pin_pending(&self, ctx: &Ctx, pin: &PinRow) -> Result<u32> {
        let refs = self.pins.db.references(&ctx.account.id)?;
        let local = local_tree(&ctx.store, pin, &ctx.exclusions);
        let mut pending = 0;
        for (path, item) in &local {
            if item.dir {
                continue;
            }
            match refs.get(path) {
                Some(r) if r.local == Some(item.sig) => {}
                _ => pending += 1,
            }
        }
        pending += refs
            .iter()
            .filter(|(p, r)| {
                within(p, &pin.path) && !r.dir && r.local.is_some() && !local.contains_key(*p)
            })
            .count() as u32;
        Ok(pending)
    }

    // Connection and sync rounds

    /// Called every second: at the check interval, looks whether each
    /// account can be reached, switches `nuage-fs` offline or online, and
    /// syncs the pinned items.
    pub(crate) async fn pins_tick(self: &std::sync::Arc<Self>) {
        let interval = {
            let st = self.state.lock().await;
            st.config
                .settings
                .offline_check_interval_secs
                .map_or(DEFAULT_CHECK_INTERVAL, Duration::from_secs)
        };
        {
            let mut last = self.pins.last_check.lock().expect("pin check lock");
            if last.is_some_and(|t| t.elapsed() < interval) {
                return;
            }
            *last = Some(Instant::now());
        }
        let inner = std::sync::Arc::clone(self);
        tokio::spawn(async move { inner.check_all().await });
    }

    async fn check_all(&self) {
        let ids: Vec<AccountId> = {
            let st = self.state.lock().await;
            st.config
                .accounts
                .iter()
                .filter(|r| {
                    let rt = st.runtime.get(&r.account.id);
                    rt.is_some_and(|rt| rt.fs.is_some())
                })
                .map(|r| r.account.id.clone())
                .collect()
        };
        for id in ids {
            if let Err(e) = self.check_account(&id).await {
                tracing::warn!(account = %id, error = %e, "offline check failed");
            }
        }
    }

    /// Whether the account's provider answers; then a sync round.
    async fn check_account(&self, id: &AccountId) -> Result<()> {
        let fs = {
            let st = self.state.lock().await;
            remote_fs(&st.record(st.resolve(id)?).account)
        };
        let online = if test_offline() {
            false
        } else {
            let rc = self.rc().await?;
            match rc.list(&fs, "").await {
                Ok(items) => {
                    self.save_listing(id, "", &items);
                    true
                }
                Err(e) if is_offline(&e) => false,
                Err(e) => {
                    tracing::debug!(account = %id, error = %e, "connection check failed");
                    true
                }
            }
        };
        self.set_online(id, online).await;
        if online {
            self.sync_account(id).await?;
        } else {
            self.count_offline_changes(id).await?;
        }
        Ok(())
    }

    /// Offline, no round runs: counts the local changes waiting for the
    /// connection.
    async fn count_offline_changes(&self, id: &AccountId) -> Result<()> {
        let ctx = {
            let st = self.state.lock().await;
            let i = st.resolve(id)?;
            self.ctx(&st, i)
        };
        for pin in self.pins.db.pins(Some(id))? {
            let pending = self.pin_pending(&ctx, &pin)?;
            let mut status = self.current_status(id, &pin.path);
            status.pending = pending;
            if pending > 0 && status.state == PinState::Available {
                status.state = PinState::ModifiedLocally;
            }
            self.set_status(id, &pin.path, status);
        }
        Ok(())
    }

    /// Switches an account's folder offline or online. Back online after
    /// an offline start, the hidden rclone mount is mounted.
    pub(crate) async fn set_online(&self, id: &AccountId, online: bool) {
        let mut st = self.state.lock().await;
        let Ok(i) = st.resolve(id) else { return };
        let name = st.record(i).account.display_name.clone();
        let runtime = st.runtime(id);
        let was_offline = runtime.offline;
        if was_offline == !online {
            return;
        }
        runtime.offline = !online;
        if let Some(fs) = &runtime.fs {
            fs.set_offline(!online);
        }
        if online {
            tracing::info!(account = %name, "back online");
            if runtime.mounted.is_none()
                && let Err(e) = self.mount_locked(&mut st, i).await
            {
                tracing::warn!(account = %name, error = %e, "cannot mount after the connection came back");
            }
        } else {
            tracing::warn!(account = %name, "offline: pinned items stay available");
        }
        drop(st);
        self.poll_status().await;
    }

    /// One sync round of an account's pins.
    pub(crate) async fn sync_account(&self, id: &AccountId) -> Result<()> {
        let round = self.pins.sync.lock().await;
        let ctx = {
            let st = self.state.lock().await;
            let i = st.resolve(id)?;
            if st.runtime.get(id).is_some_and(|r| r.offline) {
                return Ok(());
            }
            self.ctx(&st, i)
        };
        let pins = self.pins.db.pins(Some(id))?;
        if pins.is_empty() {
            return Ok(());
        }
        let rc = self.rc().await?;
        std::fs::create_dir_all(&ctx.store)?;
        std::fs::create_dir_all(self.paths.pin_staging())?;
        for pin in &pins {
            let remote = match remote_tree(&rc, &ctx.fs, pin, &ctx.exclusions).await {
                Ok(tree) => tree,
                Err(e) if is_offline(&e) => {
                    drop(round);
                    self.set_online(id, false).await;
                    return Ok(());
                }
                Err(e) => {
                    tracing::warn!(pin = %pin.path, error = %e, "cannot list a pinned item");
                    let mut s = self.current_status(id, &pin.path);
                    s.state = PinState::Error;
                    self.set_status(id, &pin.path, s);
                    continue;
                }
            };
            let status = self.sync_pin(&rc, &ctx, pin, remote).await;
            self.set_status(id, &pin.path, status);
        }
        self.save_pin_listings(&rc, &ctx, &pins).await;
        self.push_view(id).await;
        Ok(())
    }

    fn current_status(&self, id: &AccountId, path: &str) -> PinStatus {
        self.pins
            .status
            .lock()
            .expect("pin status lock")
            .get(&(id.clone(), path.to_owned()))
            .copied()
            .unwrap_or_default()
    }

    /// Saves the listings of the folders above each pin and inside pinned
    /// folders, so that they stay browsable offline.
    async fn save_pin_listings(&self, rc: &RcClient, ctx: &Ctx, pins: &[PinRow]) {
        let mut dirs: BTreeSet<String> = BTreeSet::new();
        for pin in pins {
            dirs.extend(ancestors(&pin.path));
            if pin.kind == PinKind::FolderRecursive {
                dirs.insert(pin.path.clone());
            }
        }
        // Folders inside pinned folders: from the store, which mirrors them.
        for pin in pins.iter().filter(|p| p.kind == PinKind::FolderRecursive) {
            for (path, item) in local_tree(&ctx.store, pin, &ctx.exclusions) {
                if item.dir {
                    dirs.insert(path);
                }
            }
        }
        for dir in dirs {
            match rc.list(&ctx.fs, &dir).await {
                Ok(items) => self.save_listing(&ctx.account.id, &dir, &items),
                Err(e) => {
                    tracing::debug!(%dir, error = %e, "cannot save the listing of a folder");
                }
            }
        }
    }

    /// Syncs the items under one pin; returns its new status.
    async fn sync_pin(
        &self,
        rc: &RcClient,
        ctx: &Ctx,
        pin: &PinRow,
        remote: Option<HashMap<String, RemoteItem>>,
    ) -> PinStatus {
        let id = &ctx.account.id;
        let db = &self.pins.db;
        let mut refs = db.references(id).unwrap_or_default();
        refs.retain(|p, _| within(p, &pin.path));
        let local = local_tree(&ctx.store, pin, &ctx.exclusions);
        let remote_gone = remote.is_none();
        let remote = remote.unwrap_or_default();
        let mut status = PinStatus {
            state: PinState::Available,
            ..PinStatus::default()
        };
        let mut failed = false;
        let mut missing = 0u32;
        let now = SystemTime::now();
        let quiet = |l: &LocalItem| now.duration_since(l.modified).map_or(true, |d| d >= QUIET);

        if remote_gone && refs.is_empty() && local.is_empty() {
            // Pinned, then deleted remotely before the first download.
            status.state = PinState::Error;
            return status;
        }

        // Folders first (created from the top), files, then folder
        // deletions (from the bottom).
        let mut paths: BTreeSet<String> = BTreeSet::new();
        paths.extend(remote.keys().cloned());
        paths.extend(local.keys().cloned());
        paths.extend(refs.keys().cloned());
        let is_dir = |p: &str| {
            remote
                .get(p)
                .map(|r| r.dir)
                .or(local.get(p).map(|l| l.dir))
                .or(refs.get(p).map(|r| r.dir))
                .unwrap_or(false)
        };
        let (dirs, files): (Vec<String>, Vec<String>) = paths.into_iter().partition(|p| is_dir(p));

        for path in &dirs {
            let (r, l, rf) = (remote.get(path), local.get(path), refs.get(path));
            let result: std::result::Result<(), String> = match (rf, r, l) {
                (None, Some(_), None) => std::fs::create_dir_all(ctx.store.join(path))
                    .map_err(|e| e.to_string())
                    .map(|()| self.record_dir(id, path)),
                (None, None, Some(_)) if !ctx.paused => rc
                    .mkdir(&ctx.fs, path)
                    .await
                    .map_err(|e| e.to_string())
                    .map(|()| self.record_dir(id, path)),
                (None, Some(_), Some(_)) => {
                    self.record_dir(id, path);
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(%path, error = %e, "cannot create a pinned folder");
                failed = true;
            }
        }

        for path in &files {
            let (r, l, rf) = (remote.get(path), local.get(path), refs.get(path));
            let remote_changed = match (rf.and_then(|x| x.remote.as_ref()), r) {
                (Some(a), Some(b)) => !a.same(&b.sig),
                (None, None) => false,
                _ => true,
            };
            let local_changed = rf.map(|x| x.local) != Some(l.map(|l| l.sig)) && rf.is_some();
            let action = match (rf, r, l) {
                (None, Some(_), None) => Action::Download,
                (None, None, Some(l)) => Action::Upload(quiet(l)),
                (None, Some(r), Some(l)) => {
                    let same = i128::from(r.sig.size) == l.sig.size as i128;
                    if same {
                        Action::Record
                    } else {
                        Action::Conflict(quiet(l))
                    }
                }
                (None, None, None) => Action::Nothing,
                (Some(_), _, _) => match (remote_changed, local_changed, r, l) {
                    (false, false, _, _) => Action::Nothing,
                    (true, false, Some(_), _) => Action::Download,
                    (true, false, None, _) => Action::DeleteLocal,
                    (false, true, _, Some(l)) => Action::Upload(quiet(l)),
                    (false, true, _, None) => Action::DeleteRemote,
                    (true, true, None, Some(l)) => Action::Upload(quiet(l)),
                    (true, true, Some(_), None) => Action::Download,
                    (true, true, None, None) => Action::Forget,
                    (true, true, Some(_), Some(l)) => Action::Conflict(quiet(l)),
                },
            };
            let uploads = !ctx.paused;
            let result = match action {
                Action::Nothing => Ok(()),
                Action::Record => {
                    let (r, l) = (r.expect("remote"), l.expect("local"));
                    self.record(id, path, Some(r.sig.clone()), Some(l.sig));
                    Ok(())
                }
                Action::Forget => {
                    let _ = db.remove_reference(id, path);
                    Ok(())
                }
                Action::Download => {
                    self.pin_download(rc, ctx, path, r.map(|r| r.sig.clone()))
                        .await
                }
                Action::DeleteLocal => {
                    let _ = std::fs::remove_file(ctx.store.join(path));
                    let _ = db.remove_reference(id, path);
                    Ok(())
                }
                Action::Upload(true) | Action::DeleteRemote | Action::Conflict(true)
                    if !uploads =>
                {
                    status.pending += 1;
                    Ok(())
                }
                Action::Upload(false) | Action::Conflict(false) => {
                    status.pending += 1;
                    Ok(())
                }
                Action::Upload(true) => self.pin_upload(rc, ctx, path).await,
                Action::DeleteRemote => match rc.delete_file(&ctx.fs, path).await {
                    Ok(()) => {
                        let _ = db.remove_reference(id, path);
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                },
                Action::Conflict(true) => {
                    status.state = PinState::Conflict;
                    self.conflict(rc, ctx, pin, path, r.map(|r| r.sig.clone()))
                        .await
                }
            };
            if let Err(e) = result {
                tracing::warn!(%path, error = %e, "cannot sync a pinned file");
                failed = true;
            }
            if !ctx.store.join(path).exists() && remote.contains_key(path) {
                missing += 1;
            }
        }

        for path in dirs.iter().rev() {
            let (r, l, rf) = (remote.get(path), local.get(path), refs.get(path));
            match (rf, r, l) {
                // Deleted here: deleted there once empty (its files went
                // above), else kept (new remote files came back).
                (Some(_), Some(_), None) if !ctx.paused => {
                    if rc.rmdir(&ctx.fs, path).await.is_ok() {
                        let _ = db.remove_reference(id, path);
                    } else {
                        let _ = std::fs::create_dir_all(ctx.store.join(path));
                    }
                }
                // Deleted there: deleted here once empty.
                (Some(_), None, Some(_)) => {
                    if std::fs::remove_dir(ctx.store.join(path)).is_ok() {
                        let _ = db.remove_reference(id, path);
                    } else if !ctx.paused {
                        let _ = rc.mkdir(&ctx.fs, path).await;
                    }
                }
                (Some(_), None, None) => {
                    let _ = db.remove_reference(id, path);
                }
                _ => {}
            }
        }

        status.size = local_tree(&ctx.store, pin, &ctx.exclusions)
            .values()
            .filter(|l| !l.dir)
            .map(|l| l.sig.size)
            .sum();
        if failed {
            status.state = PinState::Error;
        } else if status.state != PinState::Conflict {
            if missing > 0 {
                status.state = PinState::Downloading;
            } else if status.pending > 0 {
                status.state = PinState::ModifiedLocally;
            }
        }
        status
    }

    fn record(
        &self,
        id: &AccountId,
        path: &str,
        remote: Option<RemoteSig>,
        local: Option<LocalSig>,
    ) {
        let r = Reference {
            dir: false,
            remote,
            local,
        };
        if let Err(e) = self.pins.db.set_reference(id, path, &r) {
            tracing::warn!(error = %e, "cannot save a pinned file's state");
        }
    }

    fn record_dir(&self, id: &AccountId, path: &str) {
        let r = Reference {
            dir: true,
            remote: None,
            local: None,
        };
        if let Err(e) = self.pins.db.set_reference(id, path, &r) {
            tracing::warn!(error = %e, "cannot save a pinned folder's state");
        }
    }

    /// Downloads into the staging folder, then renames into the store:
    /// programs never see a half-written copy.
    async fn pin_download(
        &self,
        rc: &RcClient,
        ctx: &Ctx,
        path: &str,
        sig: Option<RemoteSig>,
    ) -> std::result::Result<(), String> {
        let staging = self.paths.pin_staging();
        let tmp = format!(
            "{}-{}",
            std::process::id(),
            STAGING_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let staging_text = path_text(&staging).map_err(|e| e.to_string())?;
        rc.transfer_file((&ctx.fs, path), (staging_text, &tmp), false)
            .await
            .map_err(|e| e.to_string())?;
        let dest = ctx.store.join(path);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::rename(staging.join(&tmp), &dest).map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(&dest).map_err(|e| e.to_string())?;
        let sig = match sig {
            Some(s) => Some(s),
            None => rc
                .stat(&ctx.fs, path)
                .await
                .ok()
                .flatten()
                .map(|i| remote_sig(&i)),
        };
        self.record(&ctx.account.id, path, sig, Some(local_sig(&meta)));
        tracing::info!(%path, "pinned file downloaded");
        Ok(())
    }

    async fn pin_upload(
        &self,
        rc: &RcClient,
        ctx: &Ctx,
        path: &str,
    ) -> std::result::Result<(), String> {
        let store = path_text(&ctx.store).map_err(|e| e.to_string())?;
        let before = std::fs::metadata(ctx.store.join(path)).map_err(|e| e.to_string())?;
        rc.transfer_file((store, path), (&ctx.fs, path), false)
            .await
            .map_err(|e| e.to_string())?;
        let remote = rc
            .stat(&ctx.fs, path)
            .await
            .map_err(|e| e.to_string())?
            .map(|i| remote_sig(&i));
        // Changed again while uploading: the next round sends it again.
        let after = std::fs::metadata(ctx.store.join(path)).map_err(|e| e.to_string())?;
        let local = (local_sig(&before) == local_sig(&after)).then(|| local_sig(&after));
        self.record(&ctx.account.id, path, remote, local);
        tracing::info!(%path, "pinned file uploaded");
        Ok(())
    }

    /// Both sides changed: the local version is kept under a conflict name
    /// (and uploaded), the remote one is downloaded under the original name.
    async fn conflict(
        &self,
        rc: &RcClient,
        ctx: &Ctx,
        pin: &PinRow,
        path: &str,
        sig: Option<RemoteSig>,
    ) -> std::result::Result<(), String> {
        let id = &ctx.account.id;
        let renamed = conflict_name(path, chrono::Local::now());
        std::fs::rename(ctx.store.join(path), ctx.store.join(&renamed))
            .map_err(|e| e.to_string())?;
        tracing::warn!(%path, kept = %renamed, "conflict: both versions kept");
        if let Err(e) = self.pin_upload(rc, ctx, &renamed).await {
            tracing::warn!(path = %renamed, error = %e, "cannot upload a conflict copy yet");
        }
        // A pinned file: its conflict copy is a separate, unpinned file.
        if pin.kind == PinKind::File {
            let _ = std::fs::remove_file(ctx.store.join(&renamed));
            let _ = self.pins.db.remove_reference(id, &renamed);
        }
        *self
            .pins
            .conflicts
            .lock()
            .expect("pin conflicts lock")
            .entry(id.clone())
            .or_default() += 1;
        self.emit(Event::PinStateChanged {
            account: id.clone(),
            path: renamed,
            state: PinState::Conflict,
        });
        self.pin_download(rc, ctx, path, sig).await
    }
}

#[derive(Debug, Clone, Copy)]
enum Action {
    Nothing,
    /// Same on both sides: remember it.
    Record,
    Forget,
    Download,
    DeleteLocal,
    /// `false`: changed too recently, next round.
    Upload(bool),
    DeleteRemote,
    Conflict(bool),
}

/// Removes the empty folders above `path` in the store.
fn remove_empty_parents(store: &Path, path: &str) {
    let mut dir = parent(path).to_owned();
    while !dir.is_empty() {
        if std::fs::remove_dir(store.join(&dir)).is_err() {
            break;
        }
        dir = parent(&dir).to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warns_once_per_crossing() {
        let mut warned = std::collections::HashSet::new();
        let a = AccountId::new("a");
        assert!(!crossed(&mut warned, &a, 5, Some(10)));
        assert!(crossed(&mut warned, &a, 11, Some(10)));
        assert!(!crossed(&mut warned, &a, 12, Some(10)), "once");
        assert!(!crossed(&mut warned, &a, 9, Some(10)));
        assert!(crossed(&mut warned, &a, 11, Some(10)), "again after going under");
        assert!(!crossed(&mut warned, &a, 1 << 40, None), "off");
    }

    #[test]
    fn names_and_paths() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-29T18:45:00+01:00")
            .unwrap()
            .with_timezone(&chrono::Local);
        let stamp = at.format("%Y-%m-%d %H-%M").to_string();
        assert_eq!(
            conflict_name("docs/report.docx", at),
            format!("docs/report (conflict {stamp}).docx")
        );
        assert_eq!(
            conflict_name("notes", at),
            format!("notes (conflict {stamp})")
        );
        assert_eq!(ancestors("a/b/c.txt"), ["", "a", "a/b"]);
        assert_eq!(ancestors("c.txt"), [""]);
        assert_eq!(parent("a/b"), "a");
    }
}
