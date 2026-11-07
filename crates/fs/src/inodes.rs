//! Inode numbers of `nuage-fs` and the paths they stand for.
//!
//! The kernel refers to files by inode number. `nuage-fs` hands out its own
//! numbers (rclone's are not stable across remounts) and remembers, for each
//! one, the path relative to the backing folder and how many times the
//! kernel looked it up. An entry is dropped once the kernel forgets it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::relay::Version;

pub const ROOT: u64 = 1;

#[derive(Debug)]
struct Node {
    /// Relative to the backing folder; empty for the root. `None` once the
    /// file was deleted or replaced while the kernel still knows the inode.
    path: Option<PathBuf>,
    lookups: u64,
    /// What the kernel's cached pages of the file were read from or
    /// written as (see [`Version`]).
    version: Option<Version>,
}

/// `old`, which is `from` or below it, moved to `to`.
fn moved_path(old: &Path, from: &Path, to: &Path) -> PathBuf {
    let rest = old.strip_prefix(from).expect("prefix");
    // `join("")` would add a trailing slash.
    if rest.as_os_str().is_empty() {
        to.to_owned()
    } else {
        to.join(rest)
    }
}

#[derive(Debug)]
pub struct Inodes {
    by_ino: HashMap<u64, Node>,
    by_path: HashMap<PathBuf, u64>,
    next: u64,
}

impl Default for Inodes {
    fn default() -> Self {
        let mut by_ino = HashMap::new();
        by_ino.insert(
            ROOT,
            Node {
                path: Some(PathBuf::new()),
                lookups: 1,
                version: None,
            },
        );
        let mut by_path = HashMap::new();
        by_path.insert(PathBuf::new(), ROOT);
        Self {
            by_ino,
            by_path,
            next: ROOT + 1,
        }
    }
}

impl Inodes {
    /// Path of an inode, if it still has one.
    pub fn path(&self, ino: u64) -> Option<PathBuf> {
        self.by_ino.get(&ino)?.path.clone()
    }

    /// The state of the file the last time it was opened or closed.
    pub fn version(&self, ino: u64) -> Option<Version> {
        self.by_ino.get(&ino)?.version
    }

    /// Records the state of a file just opened or closed.
    pub fn set_version(&mut self, ino: u64, version: Option<Version>) {
        if let Some(node) = self.by_ino.get_mut(&ino) {
            node.version = version;
        }
    }

    /// Inode of `path` if the kernel knows it, without counting a lookup.
    pub fn peek(&self, path: &Path) -> Option<u64> {
        self.by_path.get(path).copied()
    }

    /// Inode of `path`, created if needed, with one more lookup.
    pub fn lookup(&mut self, path: &Path) -> u64 {
        if let Some(&ino) = self.by_path.get(path) {
            if let Some(node) = self.by_ino.get_mut(&ino) {
                node.lookups += 1;
            }
            return ino;
        }
        let ino = self.next;
        self.next += 1;
        self.by_ino.insert(
            ino,
            Node {
                path: Some(path.to_owned()),
                lookups: 1,
                version: None,
            },
        );
        self.by_path.insert(path.to_owned(), ino);
        ino
    }

    /// The kernel dropped `n` references to `ino`.
    pub fn forget(&mut self, ino: u64, n: u64) {
        if ino == ROOT {
            return;
        }
        let Some(node) = self.by_ino.get_mut(&ino) else {
            return;
        };
        node.lookups = node.lookups.saturating_sub(n);
        if node.lookups == 0 {
            let node = self.by_ino.remove(&ino).expect("present");
            if let Some(path) = node.path
                && self.by_path.get(&path) == Some(&ino)
            {
                self.by_path.remove(&path);
            }
        }
    }

    /// `path` was deleted: its inode, if any, no longer has a path.
    pub fn unlinked(&mut self, path: &Path) {
        if let Some(ino) = self.by_path.remove(path)
            && let Some(node) = self.by_ino.get_mut(&ino)
        {
            node.path = None;
        }
    }

    /// `from` was renamed to `to`, with everything below it.
    pub fn renamed(&mut self, from: &Path, to: &Path) {
        // Whatever was at `to` has been replaced.
        self.unlinked(to);
        let moved: Vec<(PathBuf, u64)> = self
            .by_path
            .iter()
            .filter(|(p, _)| p.starts_with(from))
            .map(|(p, &ino)| (p.clone(), ino))
            .collect();
        for (old, ino) in moved {
            let new = moved_path(&old, from, to);
            self.by_path.remove(&old);
            self.by_path.insert(new.clone(), ino);
            if let Some(node) = self.by_ino.get_mut(&ino) {
                node.path = Some(new);
            }
        }
    }

    /// `a` and `b` swapped places (`RENAME_EXCHANGE`).
    pub fn exchanged(&mut self, a: &Path, b: &Path) {
        let tmp = PathBuf::from("\0exchange");
        self.renamed_keep(a, &tmp);
        self.renamed_keep(b, a);
        self.renamed_keep(&tmp, b);
    }

    /// Like [`renamed`](Self::renamed), but the target is known to be free.
    fn renamed_keep(&mut self, from: &Path, to: &Path) {
        let moved: Vec<(PathBuf, u64)> = self
            .by_path
            .iter()
            .filter(|(p, _)| p.starts_with(from))
            .map(|(p, &ino)| (p.clone(), ino))
            .collect();
        for (old, ino) in moved {
            let new = moved_path(&old, from, to);
            self.by_path.remove(&old);
            self.by_path.insert(new.clone(), ino);
            if let Some(node) = self.by_ino.get_mut(&ino) {
                node.path = Some(new);
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.by_ino.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_and_forget() {
        let mut t = Inodes::default();
        assert_eq!(t.path(ROOT), Some(PathBuf::new()));
        let a = t.lookup(Path::new("a"));
        assert_eq!(t.lookup(Path::new("a")), a);
        t.forget(a, 1);
        assert_eq!(t.path(a), Some(PathBuf::from("a")));
        t.forget(a, 1);
        assert_eq!(t.path(a), None);
        assert_eq!(t.len(), 1);
        t.forget(ROOT, 10);
        assert_eq!(t.path(ROOT), Some(PathBuf::new()));
    }

    #[test]
    fn rename_moves_children() {
        let mut t = Inodes::default();
        let dir = t.lookup(Path::new("dir"));
        let file = t.lookup(Path::new("dir/sub/file"));
        let other = t.lookup(Path::new("dirt"));
        let target = t.lookup(Path::new("new"));
        t.renamed(Path::new("dir"), Path::new("new"));
        assert_eq!(t.path(dir), Some(PathBuf::from("new")));
        assert_eq!(t.path(dir).unwrap().as_os_str(), "new", "no trailing slash");
        assert_eq!(t.path(file), Some(PathBuf::from("new/sub/file")));
        assert_eq!(t.path(other), Some(PathBuf::from("dirt")));
        // The replaced target keeps its inode until forgotten, without a path.
        assert_eq!(t.path(target), None);
        assert_eq!(t.peek(Path::new("new")), Some(dir));
        // Forgetting the replaced inode does not touch the new owner of the path.
        t.forget(target, 1);
        assert_eq!(t.peek(Path::new("new")), Some(dir));
    }

    #[test]
    fn exchange() {
        let mut t = Inodes::default();
        let a = t.lookup(Path::new("a"));
        let b = t.lookup(Path::new("b/x"));
        t.exchanged(Path::new("a"), Path::new("b"));
        assert_eq!(t.path(a), Some(PathBuf::from("b")));
        assert_eq!(t.path(b), Some(PathBuf::from("a/x")));
    }

    #[test]
    fn versions_follow_the_inode() {
        let mut t = Inodes::default();
        let a = t.lookup(Path::new("a"));
        let v = Version::for_test(3);
        assert_eq!(t.version(a), None);
        t.set_version(a, Some(v));
        t.renamed(Path::new("a"), Path::new("b"));
        assert_eq!(t.version(a), Some(v));
        // A new file at the old name starts without one.
        let again = t.lookup(Path::new("a"));
        assert_eq!(t.version(again), None);
        // Forgotten with the inode.
        t.forget(a, 1);
        assert_eq!(t.version(a), None);
    }

    #[test]
    fn unlink_keeps_inode() {
        let mut t = Inodes::default();
        let a = t.lookup(Path::new("a"));
        t.unlinked(Path::new("a"));
        assert_eq!(t.path(a), None);
        let again = t.lookup(Path::new("a"));
        assert_ne!(again, a);
    }
}
