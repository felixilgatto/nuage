//! File operations through the rc API, independent of the mount: they work
//! even when the account is unmounted.
//!
//! Paths are relative to the account root. After a change, the parent
//! folders are forgotten by the account's mount (`vfs/forget`), so the
//! mounted folder shows it at once.
//!
//! Transfers run as rclone `_async` jobs: they can last longer than any
//! HTTP timeout.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use nuage_core::{
    Account, Badge, Capabilities, Entry, EntryKind, Error, ExclusionSet, NativeDocPolicy,
    RemotePath, Result,
};
use nuage_rclone::{ListItem, RcClient, RcError};

use crate::Inner;
use crate::mounts::remote_fs;

/// What a file operation needs to know about an account.
#[derive(Debug)]
struct Target {
    account: Account,
    /// `"<remote>:"`.
    fs: String,
    /// The hidden rclone mount, when mounted.
    hidden: Option<PathBuf>,
    capabilities: Capabilities,
    native_docs: Option<NativeDocPolicy>,
    exclusions: ExclusionSet,
}

impl Target {
    /// The fs string of a folder of the account (`"remote:a/b"`).
    fn dir_fs(&self, path: &str) -> String {
        format!("{}{path}", self.fs)
    }

    /// `"Google Drive:Docs/a.txt"`, for messages.
    fn show(&self, path: &str) -> String {
        format!("{}:{path}", self.account.display_name)
    }
}

/// `a/b/c` → `a/b`; `c` → ``.
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// `a/b/c` → `c`.
fn base_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, n)| n)
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// Removes `.` and empty components and refuses `..`.
fn clean(path: &str) -> Result<String> {
    let mut out = Vec::new();
    for c in path.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                return Err(Error::InvalidArgument(format!(
                    "\"{path}\" is not a path inside the account."
                )));
            }
            c => out.push(c),
        }
    }
    Ok(out.join("/"))
}

/// Whether rclone says that nothing is at the path.
fn is_not_found(e: &RcError) -> bool {
    match e {
        RcError::Api { message, .. } => {
            let m = message.to_ascii_lowercase();
            m.contains("not found") || m.contains("doesn't exist") || m.contains("no such file")
        }
        _ => false,
    }
}

/// Whether the error comes from the network rather than from the request.
fn is_offline(e: &RcError) -> bool {
    match e {
        RcError::Http(_) => false,
        RcError::Api { message, .. } => {
            let m = message.to_ascii_lowercase();
            [
                "no such host",
                "dial tcp",
                "connection refused",
                "network is unreachable",
                "i/o timeout",
                "tls handshake",
                "context deadline exceeded",
            ]
            .iter()
            .any(|s| m.contains(s))
        }
        RcError::Url(_) => false,
    }
}

fn rclone_error(what: &str, e: RcError) -> Error {
    let message = match &e {
        RcError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    };
    Error::Rclone(format!("{what}: {message}"))
}

fn entry_from(item: &ListItem, target: &Target) -> Entry {
    let modified = item
        .mod_time
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Utc));
    let native_doc = (!item.is_dir)
        .then(|| {
            target
                .native_docs
                .as_ref()
                .and_then(|p| p.detect(&item.name, item.id.as_deref()))
        })
        .flatten();
    let hash = item
        .hashes
        .iter()
        .next()
        .map(|(algo, hex)| format!("{algo}:{hex}"));
    Entry {
        name: item.name.clone(),
        path: item.path.clone(),
        kind: if item.is_dir {
            EntryKind::Folder
        } else {
            EntryKind::File
        },
        size: (!item.is_dir && item.size >= 0).then_some(item.size as u64),
        modified,
        mime: item.mime_type.clone().filter(|m| m != "inode/directory"),
        hash,
        native_doc,
        badges: Vec::new(),
    }
}

/// Entries of a folder of the mount, for when the provider cannot be
/// reached: rclone answers from its directory cache. Files whose content is
/// not in the VFS cache are "online only".
fn list_cached(hidden: &Path, cache: &Path, path: &str) -> std::io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(hidden.join(path))? {
        let e = e?;
        let name = e.file_name().to_string_lossy().into_owned();
        let meta = e.metadata()?;
        let rel = join(path, &name);
        let mut badges = Vec::new();
        if !meta.is_dir() && !cache.join(&rel).exists() {
            badges.push(Badge::OnlineOnly);
        }
        out.push(Entry {
            name,
            path: rel,
            kind: if meta.is_dir() {
                EntryKind::Folder
            } else {
                EntryKind::File
            },
            size: (!meta.is_dir()).then_some(meta.len()),
            modified: meta.modified().ok().map(chrono::DateTime::from),
            mime: None,
            hash: None,
            native_doc: None,
            badges,
        });
    }
    Ok(out)
}

/// Folders first, then by name, ignoring case.
fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.is_dir()
            .cmp(&a.is_dir())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// Whether `path` is `dir` or inside it.
fn within(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

impl Inner {
    async fn target(&self, id: &nuage_core::AccountId) -> Result<Target> {
        let st = self.state.lock().await;
        let record = st.record(st.resolve(id)?);
        let account = record.account.clone();
        let provider = self.registry.get(&account.provider_id).ok_or_else(|| {
            Error::Internal(format!("unknown provider \"{}\"", account.provider_id))
        })?;
        let runtime = st.runtime.get(&account.id);
        Ok(Target {
            fs: remote_fs(&account),
            hidden: runtime
                .and_then(|r| r.mounted.as_ref())
                .map(|plan| plan.rclone_mount_point.clone()),
            capabilities: runtime
                .and_then(|r| r.capabilities.clone())
                .unwrap_or_else(|| provider.static_capabilities()),
            native_docs: provider.native_docs(),
            exclusions: ExclusionSet::new(&record.exclusion_rules()),
            account,
        })
    }

    /// Asks the mount of `target` to list `dirs` again.
    async fn forget(&self, rc: &RcClient, target: &Target, dirs: &[&str]) {
        if target.hidden.is_none() {
            return;
        }
        let dirs: BTreeSet<String> = dirs.iter().map(|d| (*d).to_owned()).collect();
        // `vfs/forget` names directories with `dir…` keys; the root has no
        // name, so forgetting it means forgetting everything.
        let result = if dirs.contains("") {
            rc.call::<_, serde::de::IgnoredAny>(
                "vfs/forget",
                &serde_json::json!({ "fs": target.fs }),
            )
            .await
            .map(drop)
        } else {
            rc.vfs_forget_dirs(&target.fs, &dirs.into_iter().collect::<Vec<_>>())
                .await
        };
        if let Err(e) = result {
            tracing::debug!(error = %e, "vfs/forget failed");
        }
    }

    async fn stat(&self, rc: &RcClient, target: &Target, path: &str) -> Result<Option<ListItem>> {
        if path.is_empty() {
            // The root always exists.
            return Ok(Some(ListItem {
                is_dir: true,
                ..ListItem::default()
            }));
        }
        rc.stat(&target.fs, path)
            .await
            .map_err(|e| rclone_error(&format!("cannot read \"{}\"", target.show(path)), e))
    }

    async fn existing(&self, rc: &RcClient, target: &Target, path: &str) -> Result<ListItem> {
        self.stat(rc, target, path)
            .await?
            .ok_or_else(|| Error::NotFound(format!("\"{}\" does not exist.", target.show(path))))
    }

    pub(crate) async fn list_files(&self, at: &RemotePath) -> Result<Vec<Entry>> {
        let target = self.target(&at.account).await?;
        let path = clean(&at.path)?;
        let rc = self.rc().await?;
        let mut entries = match rc.list(&target.fs, &path).await {
            Ok(items) => items.iter().map(|i| entry_from(i, &target)).collect(),
            Err(e) if is_not_found(&e) => {
                return Err(Error::NotFound(format!(
                    "\"{}\" does not exist.",
                    target.show(&path)
                )));
            }
            Err(e) => match (&target.hidden, is_offline(&e)) {
                (Some(hidden), true) => {
                    let cache =
                        self.paths.remote_cache_dirs(&target.account.remote_name)[0].clone();
                    let hidden = hidden.clone();
                    let p = path.clone();
                    tokio::task::spawn_blocking(move || list_cached(&hidden, &cache, &p))
                        .await
                        .map_err(|e| Error::Internal(e.to_string()))?
                        .map_err(|_| rclone_error("cannot list (offline)", e))?
                }
                _ => {
                    return Err(rclone_error(
                        &format!("cannot list \"{}\"", target.show(&path)),
                        e,
                    ));
                }
            },
        };

        let queue: Vec<String> = if target.hidden.is_some() {
            rc.vfs_queue(&target.fs)
                .await
                .map(|q| q.into_iter().map(|i| i.name).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        for entry in &mut entries {
            if target.exclusions.is_excluded(&entry.path) {
                entry.badges.push(Badge::NotSynced);
            }
            let uploading = queue.iter().any(|q| {
                if entry.is_dir() {
                    within(q, &entry.path) && q != &entry.path
                } else {
                    q == &entry.path
                }
            });
            if uploading {
                entry.badges.push(Badge::Uploading);
            }
        }
        // Files written in the mount but not uploaded yet are not listed by
        // the provider.
        for q in &queue {
            if parent(q) == path && !entries.iter().any(|e| &e.path == q) {
                entries.push(Entry {
                    name: base_name(q).to_owned(),
                    path: q.clone(),
                    kind: EntryKind::File,
                    size: None,
                    modified: None,
                    mime: None,
                    hash: None,
                    native_doc: None,
                    badges: vec![Badge::Uploading],
                });
            }
        }
        sort(&mut entries);
        Ok(entries)
    }

    pub(crate) async fn mkdir(&self, at: &RemotePath) -> Result<()> {
        let target = self.target(&at.account).await?;
        let path = clean(&at.path)?;
        if path.is_empty() {
            return Err(Error::AlreadyExists(
                "The account root exists already.".to_owned(),
            ));
        }
        let rc = self.rc().await?;
        if self.stat(&rc, &target, &path).await?.is_some() {
            return Err(Error::AlreadyExists(format!(
                "\"{}\" exists already.",
                target.show(&path)
            )));
        }
        rc.mkdir(&target.fs, &path)
            .await
            .map_err(|e| rclone_error(&format!("cannot create \"{}\"", target.show(&path)), e))?;
        self.forget(&rc, &target, &[parent(&path)]).await;
        Ok(())
    }

    /// Moves or copies (`moving`) a file or a folder, also between
    /// accounts. When `to` is an existing folder, the item goes inside it.
    pub(crate) async fn transfer(
        &self,
        from: &RemotePath,
        to: &RemotePath,
        moving: bool,
    ) -> Result<()> {
        let src = self.target(&from.account).await?;
        let dst = self.target(&to.account).await?;
        let from_path = clean(&from.path)?;
        let mut to_path = clean(&to.path)?;
        if from_path.is_empty() {
            return Err(Error::InvalidArgument(
                "The account root cannot be moved or copied; name a file or folder in it."
                    .to_owned(),
            ));
        }
        let rc = self.rc().await?;
        let item = self.existing(&rc, &src, &from_path).await?;
        if let Some(existing) = self.stat(&rc, &dst, &to_path).await? {
            if existing.is_dir {
                to_path = join(&to_path, base_name(&from_path));
            } else {
                return Err(Error::AlreadyExists(format!(
                    "\"{}\" exists already.",
                    dst.show(&to_path)
                )));
            }
            if self.stat(&rc, &dst, &to_path).await?.is_some() {
                return Err(Error::AlreadyExists(format!(
                    "\"{}\" exists already.",
                    dst.show(&to_path)
                )));
            }
        }
        let same_account = src.account.id == dst.account.id;
        if same_account && to_path == from_path {
            return Ok(());
        }
        if same_account && item.is_dir && within(&to_path, &from_path) {
            return Err(Error::InvalidArgument(format!(
                "\"{}\" cannot go inside itself.",
                src.show(&from_path)
            )));
        }
        let what = if moving { "cannot move" } else { "cannot copy" };
        let result = if item.is_dir {
            rc.transfer_dir(&src.dir_fs(&from_path), &dst.dir_fs(&to_path), moving)
                .await
        } else {
            rc.transfer_file((&src.fs, &from_path), (&dst.fs, &to_path), moving)
                .await
        };
        result.map_err(|e| rclone_error(&format!("{what} \"{}\"", src.show(&from_path)), e))?;
        if moving {
            // `sync/move` leaves the emptied source folder.
            if item.is_dir {
                let _ = rc.rmdir(&src.fs, &from_path).await;
            }
            self.forget(&rc, &src, &[parent(&from_path)]).await;
        }
        self.forget(&rc, &dst, &[parent(&to_path)]).await;
        Ok(())
    }

    pub(crate) async fn delete(&self, at: &RemotePath, recursive: bool) -> Result<()> {
        let target = self.target(&at.account).await?;
        let path = clean(&at.path)?;
        if path.is_empty() {
            return Err(Error::InvalidArgument(
                "The account root cannot be deleted. Remove the account instead (nuage account remove)."
                    .to_owned(),
            ));
        }
        let rc = self.rc().await?;
        let item = self.existing(&rc, &target, &path).await?;
        let result = if item.is_dir {
            if recursive {
                rc.purge(&target.fs, &path).await
            } else {
                match rc.list(&target.fs, &path).await {
                    Ok(items) if !items.is_empty() => {
                        return Err(Error::InvalidArgument(format!(
                            "\"{}\" is a folder that is not empty. Delete it with everything in it with --recursive.",
                            target.show(&path)
                        )));
                    }
                    _ => rc.rmdir(&target.fs, &path).await,
                }
            }
        } else {
            rc.delete_file(&target.fs, &path).await
        };
        result
            .map_err(|e| rclone_error(&format!("cannot delete \"{}\"", target.show(&path)), e))?;
        self.forget(&rc, &target, &[parent(&path)]).await;
        Ok(())
    }

    /// Uploads a local file or folder. When `to` is an existing folder, it
    /// goes inside it.
    pub(crate) async fn upload(&self, local: &Path, to: &RemotePath) -> Result<()> {
        let target = self.target(&to.account).await?;
        let mut to_path = clean(&to.path)?;
        let local = absolute_local(local)?;
        let meta = std::fs::metadata(&local)
            .map_err(|e| Error::NotFound(format!("cannot read \"{}\": {e}", local.display())))?;
        let name = local_name(&local)?;
        let rc = self.rc().await?;
        match self.stat(&rc, &target, &to_path).await? {
            Some(existing) if existing.is_dir => to_path = join(&to_path, &name),
            Some(_) if meta.is_dir() => {
                return Err(Error::AlreadyExists(format!(
                    "\"{}\" is a file.",
                    target.show(&to_path)
                )));
            }
            _ => {}
        }
        let what = format!("cannot upload \"{}\"", local.display());
        if meta.is_dir() {
            rc.transfer_dir(path_str(&local)?, &target.dir_fs(&to_path), false)
                .await
        } else {
            let dir = local.parent().unwrap_or(Path::new("/"));
            rc.transfer_file((path_str(dir)?, &name), (&target.fs, &to_path), false)
                .await
        }
        .map_err(|e| rclone_error(&what, e))?;
        self.forget(&rc, &target, &[parent(&to_path)]).await;
        Ok(())
    }

    /// Downloads a file or folder. When `local` is an existing folder, it
    /// goes inside it.
    pub(crate) async fn download(&self, from: &RemotePath, local: &Path) -> Result<()> {
        let target = self.target(&from.account).await?;
        let path = clean(&from.path)?;
        let mut local = absolute_local(local)?;
        let rc = self.rc().await?;
        let item = self.existing(&rc, &target, &path).await?;
        if local.is_dir() {
            let name = if path.is_empty() {
                target.account.display_name.clone()
            } else {
                base_name(&path).to_owned()
            };
            local = local.join(name);
        }
        let what = format!("cannot download \"{}\"", target.show(&path));
        if item.is_dir {
            std::fs::create_dir_all(&local)?;
            rc.transfer_dir(&target.dir_fs(&path), path_str(&local)?, false)
                .await
        } else {
            let dir = local.parent().unwrap_or(Path::new("/"));
            if !dir.is_dir() {
                return Err(Error::NotFound(format!(
                    "The folder \"{}\" does not exist.",
                    dir.display()
                )));
            }
            let name = local_name(&local)?;
            rc.transfer_file((&target.fs, &path), (path_str(dir)?, &name), false)
                .await
        }
        .map_err(|e| rclone_error(&what, e))
    }

    pub(crate) async fn public_link(&self, at: &RemotePath) -> Result<String> {
        let target = self.target(&at.account).await?;
        if !target.capabilities.public_link {
            return Err(Error::Unsupported(format!(
                "{} has no share links.",
                target.account.display_name
            )));
        }
        let path = clean(&at.path)?;
        let rc = self.rc().await?;
        self.existing(&rc, &target, &path).await?;
        rc.public_link(&target.fs, &path).await.map_err(|e| {
            rclone_error(
                &format!("cannot create a link to \"{}\"", target.show(&path)),
                e,
            )
        })
    }
}

fn absolute_local(local: &Path) -> Result<PathBuf> {
    if !local.is_absolute() {
        return Err(Error::InvalidArgument(format!(
            "\"{}\" must be an absolute path.",
            local.display()
        )));
    }
    Ok(crate::mounts::normalize(local))
}

fn local_name(local: &Path) -> Result<String> {
    local
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .ok_or_else(|| Error::InvalidArgument(format!("\"{}\" has no file name.", local.display())))
}

fn path_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        Error::InvalidArgument(format!("\"{}\" is not valid UTF-8.", path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(clean("/a//./b/").unwrap(), "a/b");
        assert_eq!(clean("").unwrap(), "");
        assert!(clean("a/../b").is_err());
        assert_eq!(parent("a/b/c"), "a/b");
        assert_eq!(parent("c"), "");
        assert_eq!(base_name("a/b/c"), "c");
        assert_eq!(join("", "x"), "x");
        assert_eq!(join("a", "x"), "a/x");
        assert!(within("a/b", "a"));
        assert!(within("a", "a"));
        assert!(!within("ab", "a"));
        assert!(within("anything", ""));
    }

    #[test]
    fn entries_sorted_folders_first() {
        let e = |name: &str, kind| Entry {
            name: name.into(),
            path: name.into(),
            kind,
            size: None,
            modified: None,
            mime: None,
            hash: None,
            native_doc: None,
            badges: Vec::new(),
        };
        let mut list = vec![
            e("b.txt", EntryKind::File),
            e("Zed", EntryKind::Folder),
            e("A.txt", EntryKind::File),
            e("alpha", EntryKind::Folder),
        ];
        sort(&mut list);
        let names: Vec<_> = list.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alpha", "Zed", "A.txt", "b.txt"]);
    }
}
