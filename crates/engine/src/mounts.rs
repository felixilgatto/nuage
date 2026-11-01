//! Mounts: `mount/mount`, `mount/unmount`, mount point validation and
//! cleanup of mounts left behind by a crashed daemon.

use std::path::{Component, Path, PathBuf};

use nuage_core::{Account, AccountId, Error, Event, Result, VfsOptions};
use nuage_rclone::{MountOpt, MountRequest, VfsOpt};

use crate::units::rclone_duration;
use crate::{Inner, Paths, State};

/// rclone's write-back delay for every mount: uploads never start on their
/// own; `UploadControl` releases them unless the account is paused.
pub const HOLD_WRITE_BACK: &str = "8760h";

/// Where an account is mounted: `nuage-fs` at the user's folder, on top of
/// a hidden rclone mount (`data/mounts/<account id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPlan {
    pub account: AccountId,
    /// Folder chosen by the user (`Account::mount_point`).
    pub user_mount_point: PathBuf,
    /// Where rclone itself mounts.
    pub rclone_mount_point: PathBuf,
}

impl MountPlan {
    pub fn layered(account: &Account, paths: &Paths) -> Self {
        Self {
            account: account.id.clone(),
            user_mount_point: account.mount_point.clone(),
            rclone_mount_point: paths.hidden_mounts().join(account.id.as_str()),
        }
    }
}

/// The rclone fs string of an account's remote (`"<remote>:"`).
pub fn remote_fs(account: &Account) -> String {
    format!("{}:", account.remote_name)
}

/// Builds the `mount/mount` request of an account.
pub fn mount_request(
    account: &Account,
    plan: &MountPlan,
    recommended: &VfsOptions,
    cache_max_age_secs: Option<u64>,
) -> Result<MountRequest> {
    let mount_point = path_str(&plan.rclone_mount_point)?.to_owned();
    let cache_mode = match recommended.cache_mode {
        nuage_core::CacheMode::Off => "off",
        nuage_core::CacheMode::Minimal => "minimal",
        nuage_core::CacheMode::Writes => "writes",
        // Pause relies on the disk cache holding the writes.
        nuage_core::CacheMode::Full => "full",
    };
    Ok(MountRequest {
        fs: remote_fs(account),
        mount_point,
        vfs_opt: VfsOpt {
            cache_mode: cache_mode.to_owned(),
            cache_max_size: Some(account.cache_limit),
            cache_max_age: cache_max_age_secs
                .or(recommended.cache_max_age_secs)
                .map(rclone_duration),
            write_back: Some(HOLD_WRITE_BACK.to_owned()),
            dir_cache_time: recommended.dir_cache_time_secs.map(rclone_duration),
            poll_interval: recommended.poll_interval_secs.map(rclone_duration),
        },
        mount_opt: MountOpt {
            volume_name: Some(account.display_name.clone()),
        },
    })
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        Error::InvalidArgument(format!(
            "The folder {} has a name that is not valid UTF-8. Choose another folder.",
            path.display()
        ))
    })
}

/// Lexically normalises an absolute path (`.` and `..` removed).
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// Checks a folder chosen for an account and returns its normalised form.
///
/// The folder must be absolute, empty or missing, outside Nuage's own
/// directories, and neither inside nor around another account's folder.
pub fn validate_mount_point(path: &Path, paths: &Paths, others: &[&Account]) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(Error::InvalidArgument(format!(
            "The mount folder must be an absolute path, not {}.",
            path.display()
        )));
    }
    let path = normalize(path);
    path_str(&path)?;
    if path.parent().is_none() {
        return Err(Error::InvalidArgument(
            "The root folder cannot be used as a mount folder.".to_owned(),
        ));
    }
    if is_document_portal_path(&path) {
        return Err(Error::InvalidArgument(format!(
            "{} is a document portal path, not a real folder. Choose a folder in your home directory.",
            path.display()
        )));
    }
    if paths.in_flatpak && !path.starts_with(&paths.home) {
        return Err(Error::InvalidArgument(format!(
            "Nuage cannot reach {} from its sandbox. Allow it with:\n  flatpak override --user --filesystem={} {}\nthen try again.",
            path.display(),
            path.display(),
            nuage_core::APP_ID
        )));
    }
    for dir in paths.private_dirs() {
        if overlaps(&path, dir) {
            return Err(Error::InvalidArgument(format!(
                "{} overlaps Nuage's own data folder {}. Choose another folder.",
                path.display(),
                dir.display()
            )));
        }
    }
    for other in others {
        if overlaps(&path, &other.mount_point) {
            return Err(Error::AlreadyExists(format!(
                "{} overlaps the folder of the account \"{}\" ({}). Choose another folder.",
                path.display(),
                other.display_name,
                other.mount_point.display()
            )));
        }
    }
    match std::fs::read_dir(&path) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return Err(Error::InvalidArgument(format!(
                    "{} is not empty. Choose an empty or new folder.",
                    path.display()
                )));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => {
            return Err(Error::InvalidArgument(format!(
                "{} is a file, not a folder.",
                path.display()
            )));
        }
        Err(e) => {
            return Err(Error::Io(format!("cannot read {}: {e}", path.display())));
        }
    }
    Ok(path)
}

/// `/run/user/<uid>/doc/…`: paths handed out by the document portal.
fn is_document_portal_path(path: &Path) -> bool {
    let mut c = path.components().skip(1);
    matches!(
        (c.next(), c.next(), c.next(), c.next()),
        (Some(a), Some(b), Some(_), Some(d))
            if a.as_os_str() == "run" && b.as_os_str() == "user" && d.as_os_str() == "doc"
    )
}

/// FUSE mount points currently in the kernel's mount table, from
/// `/proc/self/mountinfo`.
pub fn fuse_mounts() -> Vec<PathBuf> {
    std::fs::read_to_string("/proc/self/mountinfo")
        .map(|text| parse_fuse_mounts(&text))
        .unwrap_or_default()
}

fn parse_fuse_mounts(mountinfo: &str) -> Vec<PathBuf> {
    mountinfo
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split(' ').collect();
            let mount_point = fields.get(4)?;
            let sep = fields.iter().position(|f| *f == "-")?;
            let fstype = fields.get(sep + 1)?;
            fstype
                .starts_with("fuse")
                .then(|| PathBuf::from(unescape_mountinfo(mount_point)))
        })
        .collect()
}

/// Decodes the `\NNN` octal escapes of mountinfo (space, tab, `\`, newline).
fn unescape_mountinfo(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)))
            .and_then(|d| u8::from_str_radix(std::str::from_utf8(d).ok()?, 8).ok());
        if bytes[i] == b'\\'
            && let Some(v) = octal
        {
            out.push(v);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Detaches a FUSE mount with `fusermount3 -u -z` (lazy: works even when
/// the mount is stale or busy).
pub async fn force_unmount(path: &Path) -> Result<()> {
    let mut last = String::new();
    for tool in ["fusermount3", "fusermount"] {
        match tokio::process::Command::new(tool)
            .arg("-u")
            .arg("-z")
            .arg(path)
            .output()
            .await
        {
            Ok(out) if out.status.success() => return Ok(()),
            Ok(out) => last = String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            Err(e) => last = format!("cannot run {tool}: {e}"),
        }
    }
    Err(Error::Io(format!(
        "cannot unmount {}: {last}",
        path.display()
    )))
}

/// Turns an rclone mount error into a message that says what to do.
fn mount_error(account: &Account, e: &nuage_rclone::RcError) -> String {
    let raw = match e {
        nuage_rclone::RcError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    };
    if raw.contains("not empty") {
        format!(
            "Cannot mount \"{}\": the folder {} is not empty. Move its content elsewhere, then run `nuage mount \"{}\"`.",
            account.display_name,
            account.mount_point.display(),
            account.display_name
        )
    } else if raw.contains("fusermount") || raw.contains("/dev/fuse") {
        format!(
            "Cannot mount \"{}\": FUSE is not available ({raw}). Install fuse3 and check that /dev/fuse exists.",
            account.display_name
        )
    } else {
        format!("Cannot mount \"{}\": {raw}", account.display_name)
    }
}

impl Inner {
    /// `nuage mount`: mounts now and at every daemon start.
    pub(crate) async fn mount_command(&self, id: &AccountId) -> Result<()> {
        let mut st = self.state.lock().await;
        let i = st.resolve(id)?;
        if !st.config.accounts[i].auto_mount {
            st.config.accounts[i].auto_mount = true;
            self.save(&st)?;
        }
        self.mount_locked(&mut st, i).await
    }

    /// `nuage unmount`: unmounts and stays unmounted across restarts.
    pub(crate) async fn unmount_command(&self, id: &AccountId) -> Result<()> {
        let mut st = self.state.lock().await;
        let i = st.resolve(id)?;
        if st.config.accounts[i].auto_mount {
            st.config.accounts[i].auto_mount = false;
            self.save(&st)?;
        }
        let account = st.record(i).account.clone();
        self.unmount_locked(&mut st, &account).await
    }

    /// Mounts account `i` (both layers); does nothing if it is mounted.
    ///
    /// If `nuage-fs` is still mounted at the user's folder (rclone was
    /// restarted), only the hidden rclone mount is mounted again.
    pub(crate) async fn mount_locked(&self, st: &mut State, i: usize) -> Result<()> {
        let account = st.record(i).account.clone();
        if st.is_mounted(&account.id) {
            return Ok(());
        }
        let result = self.do_mount(st, &account).await;
        let runtime = st.runtime(&account.id);
        match &result {
            Ok(plan) => {
                runtime.mounted = Some(plan.clone());
                runtime.mount_error = None;
                runtime.quota_at = None;
                if let Some(fs) = &runtime.fs {
                    fs.set_available(true);
                }
                self.emit(Event::MountChanged {
                    account: account.id.clone(),
                    mounted: true,
                });
                tracing::info!(account = %account.display_name, at = %plan.user_mount_point.display(), "mounted");
            }
            Err(e) => runtime.mount_error = Some(e.to_string()),
        }
        result.map(drop)
    }

    async fn do_mount(&self, st: &mut State, account: &Account) -> Result<MountPlan> {
        let provider = self.registry.get(&account.provider_id).ok_or_else(|| {
            Error::Unsupported(format!(
                "The provider \"{}\" of \"{}\" is not available in this build of Nuage.",
                account.provider_id, account.display_name
            ))
        })?;
        let rc = self.rc().await?;
        let plan = MountPlan::layered(account, &self.paths);
        let kernel = fuse_mounts();

        // Hidden rclone mount.
        let hidden = &plan.rclone_mount_point;
        if kernel.contains(hidden) {
            tracing::info!(path = %hidden.display(), "detaching a stale mount");
            force_unmount(hidden).await?;
        }
        std::fs::create_dir_all(hidden)?;
        let request = mount_request(
            account,
            &plan,
            &provider.recommended_vfs_opts(),
            st.config.settings.cache_max_age_secs,
        )?;
        rc.mount(&request)
            .await
            .map_err(|e| Error::Rclone(mount_error(account, &e)))?;

        // nuage-fs at the user's folder, unless it is still there.
        if st.runtime(&account.id).fs.is_none() {
            let user = plan.user_mount_point.clone();
            if kernel.contains(&user) {
                tracing::info!(path = %user.display(), "detaching a stale mount");
                force_unmount(&user).await?;
            }
            let mounted = match check_empty(account, &user) {
                Ok(()) => {
                    let backing = hidden.clone();
                    let config = nuage_fs::MountConfig {
                        name: account.remote_name.clone(),
                        ..nuage_fs::MountConfig::default()
                    };
                    tokio::task::spawn_blocking(move || nuage_fs::mount(&backing, &user, &config))
                        .await
                        .map_err(|e| Error::Internal(e.to_string()))?
                        .map_err(|e| {
                            Error::Io(format!(
                                "Cannot mount \"{}\" at {}: {e}",
                                account.display_name,
                                plan.user_mount_point.display()
                            ))
                        })
                }
                Err(e) => Err(e),
            };
            match mounted {
                Ok(fs) => st.runtime(&account.id).fs = Some(fs),
                Err(e) => {
                    // Do not leave the hidden mount behind.
                    if let Err(u) = rc.unmount(path_str(hidden)?).await {
                        tracing::warn!(error = %u, "cannot unmount the hidden mount");
                    }
                    return Err(e);
                }
            }
        }

        let mut caps = provider.static_capabilities();
        match rc.fsinfo(&request.fs).await {
            Ok(info) => caps.merge_fsinfo_features(&info.features),
            Err(e) => {
                tracing::warn!(account = %account.display_name, error = %e, "operations/fsinfo failed")
            }
        }
        st.runtime(&account.id).capabilities = Some(caps);
        Ok(plan)
    }

    /// Unmounts both layers of an account. Writes not uploaded yet stay in
    /// the cache and are queued again at the next mount.
    pub(crate) async fn unmount_locked(&self, st: &mut State, account: &Account) -> Result<()> {
        let runtime = st.runtime(&account.id);
        let fs = runtime.fs.take();
        let plan = runtime.mounted.take();
        if fs.is_none() && plan.is_none() {
            return Ok(());
        }
        // The user's folder first: nothing reaches the hidden mount anymore.
        if let Some(fs) = fs {
            let at = fs.mount_point().to_owned();
            let result = tokio::task::spawn_blocking(move || fs.unmount())
                .await
                .map_err(|e| Error::Internal(e.to_string()))?;
            if let Err(e) = result {
                tracing::warn!(error = %e, path = %at.display(), "unmounting nuage-fs failed, detaching");
                if fuse_mounts().contains(&at) {
                    force_unmount(&at).await?;
                }
            }
        }
        let hidden = MountPlan::layered(account, &self.paths).rclone_mount_point;
        let result = match self.rc().await {
            Ok(rc) => rc
                .unmount(path_str(&hidden)?)
                .await
                .map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        if let Err(e) = result
            && fuse_mounts().contains(&hidden)
        {
            tracing::warn!(error = %e, "mount/unmount failed, detaching");
            force_unmount(&hidden).await?;
        }
        st.runtime(&account.id).disk_cache = None;
        self.emit(Event::MountChanged {
            account: account.id.clone(),
            mounted: false,
        });
        tracing::info!(account = %account.display_name, "unmounted");
        Ok(())
    }

    /// rclone stopped: the hidden mounts are gone. `nuage-fs` stays mounted
    /// and makes requests wait until [`remount_all`](Self::remount_all).
    pub(crate) fn hidden_mounts_lost(&self, st: &mut State) {
        for runtime in st.runtime.values_mut() {
            runtime.mounted = None;
            if let Some(fs) = &runtime.fs {
                fs.set_available(false);
            }
        }
    }

    /// Detaches mounts left by a crash, then mounts every account marked
    /// for it. Failures are recorded per account, not returned.
    pub(crate) async fn remount_all(&self) {
        let mut st = self.state.lock().await;
        let kernel = fuse_mounts();
        // The user's folders still served by this daemon are not orphans.
        let ours: Vec<PathBuf> = st
            .runtime
            .values()
            .filter_map(|r| r.fs.as_ref().map(|fs| fs.mount_point().to_owned()))
            .collect();
        let mut stale: Vec<PathBuf> = st
            .config
            .accounts
            .iter()
            .map(|r| r.account.mount_point.clone())
            .filter(|p| kernel.contains(p) && !ours.contains(p))
            .collect();
        let hidden = self.paths.hidden_mounts();
        stale.extend(kernel.iter().filter(|p| p.starts_with(&hidden)).cloned());
        for path in stale {
            tracing::info!(path = %path.display(), "cleaning up an orphan mount");
            if let Err(e) = force_unmount(&path).await {
                tracing::warn!(error = %e, "orphan cleanup failed");
            }
        }
        for runtime in st.runtime.values_mut() {
            runtime.mounted = None;
        }
        for i in 0..st.config.accounts.len() {
            if st.record(i).auto_mount
                && let Err(e) = self.mount_locked(&mut st, i).await
            {
                tracing::warn!(error = %e, "mount failed");
            }
        }
    }

    pub(crate) async fn unmount_all(&self) {
        let mut st = self.state.lock().await;
        let accounts: Vec<Account> = st
            .config
            .accounts
            .iter()
            .map(|r| r.account.clone())
            .collect();
        for account in accounts {
            if let Err(e) = self.unmount_locked(&mut st, &account).await {
                tracing::warn!(account = %account.display_name, error = %e, "unmount failed");
            }
        }
    }
}

/// The user's folder must be empty when `nuage-fs` mounts over it, or its
/// content would be hidden.
fn check_empty(account: &Account, path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    if std::fs::read_dir(path)?.next().is_some() {
        return Err(Error::InvalidArgument(format!(
            "Cannot mount \"{}\": the folder {} is not empty. Move its content elsewhere, then run `nuage mount \"{}\"`.",
            account.display_name,
            path.display(),
            account.display_name
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nuage_core::AccountId;

    fn account(name: &str, mount_point: &str) -> Account {
        Account {
            id: AccountId::new(name),
            provider_id: "local".into(),
            display_name: name.into(),
            remote_name: format!("nuage-{name}"),
            mount_point: mount_point.into(),
            cache_limit: 1 << 30,
        }
    }

    #[test]
    fn mountinfo() {
        let text = "\
36 35 98:0 / /mnt1 rw,noatime master:1 - ext3 /dev/root rw,errors=continue
80 25 0:52 / /home/me/Cloud/Google\\040Drive rw,nosuid,nodev - fuse.rclone gdrive: rw,user_id=1000
81 25 0:53 / /run/user/1000/doc rw - fuse.portal portal rw
";
        assert_eq!(
            parse_fuse_mounts(text),
            vec![
                PathBuf::from("/home/me/Cloud/Google Drive"),
                PathBuf::from("/run/user/1000/doc"),
            ]
        );
        assert_eq!(unescape_mountinfo("a\\134b"), "a\\b");
        assert_eq!(unescape_mountinfo("trailing\\04"), "trailing\\04");
    }

    #[test]
    fn mount_point_rules() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::under(root.path());
        let home = &paths.home;
        let drive = account("Drive", home.join("Cloud/Drive").to_str().unwrap());
        let others = [&drive];

        let ok = validate_mount_point(&home.join("Cloud/./Work"), &paths, &others).unwrap();
        assert_eq!(ok, home.join("Cloud/Work"));

        let refused = [
            PathBuf::from("relative/path"),
            PathBuf::from("/"),
            PathBuf::from("/run/user/1000/doc/abcd/Drive"),
            home.join("Cloud/Drive"),
            home.join("Cloud/Drive/inner"),
            home.join("Cloud"),
            paths.cache_dir.join("x"),
            paths.data_dir.clone(),
        ];
        for path in refused {
            assert!(
                validate_mount_point(&path, &paths, &others).is_err(),
                "{}",
                path.display()
            );
        }

        let full = home.join("full");
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("file"), "x").unwrap();
        assert!(validate_mount_point(&full, &paths, &others).is_err());
        assert!(validate_mount_point(&full.join("file"), &paths, &others).is_err());

        let flatpak = Paths {
            in_flatpak: true,
            ..paths.clone()
        };
        let err = validate_mount_point(Path::new("/mnt/data/Drive"), &flatpak, &[]).unwrap_err();
        assert!(err.to_string().contains("flatpak override"), "{err}");
    }

    #[test]
    fn request_shape() {
        let a = account("Drive", "/home/me/Cloud/Drive");
        let paths = Paths::under(Path::new("/data"));
        let plan = MountPlan::layered(&a, &paths);
        assert_eq!(plan.user_mount_point, Path::new("/home/me/Cloud/Drive"));
        let req = mount_request(&a, &plan, &VfsOptions::default(), Some(3600)).unwrap();
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["fs"], "nuage-Drive:");
        assert_eq!(json["mountPoint"], "/data/data/mounts/Drive");
        assert_eq!(json["vfsOpt"]["CacheMode"], "full");
        assert_eq!(json["vfsOpt"]["CacheMaxSize"], 1u64 << 30);
        assert_eq!(json["vfsOpt"]["CacheMaxAge"], "3600s");
        assert_eq!(json["vfsOpt"]["WriteBack"], HOLD_WRITE_BACK);
        assert!(json["vfsOpt"].get("PollInterval").is_none());
        assert_eq!(json["mountOpt"]["VolumeName"], "Drive");
    }
}
