//! What the engine tells `nuage-fs` about offline pinning (v1.1).
//!
//! Pinned items live in the pin store (`data/pins/<account>`), once their
//! copy is there; the engine syncs them through rclone. While offline,
//! the items that are not pinned are listed from the engine's metadata
//! (the last listings it saved), and opening them fails at once.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// An item of a saved listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: OsString,
    pub dir: bool,
    pub size: u64,
    pub modified: SystemTime,
}

/// The pins of an account, as `nuage-fs` needs them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PinView {
    /// Pinned paths, relative to the account root. A folder covers
    /// everything inside it.
    pub pins: Vec<PathBuf>,
    /// Pinned items deleted locally and not yet remotely: hidden.
    pub hidden: HashSet<PathBuf>,
    /// Saved listings, by folder (relative; the root is `""`), for offline.
    pub listings: HashMap<PathBuf, Vec<Listed>>,
}

impl PinView {
    /// Whether `rel` is pinned: a pin itself, or inside a pinned folder.
    ///
    /// Also true of the temporary files that editors save next to a pinned
    /// file before renaming them over it (`db.kdbx.Ab12Cd`, `report.odt~`):
    /// the rename then stays inside the store.
    pub fn covers(&self, rel: &Path) -> bool {
        if rel.as_os_str().is_empty() {
            return self.pins.iter().any(|p| p.as_os_str().is_empty());
        }
        self.pins.iter().any(|pin| {
            rel.starts_with(pin) || {
                let (Some(pin_name), Some(name)) = (pin.file_name(), rel.file_name()) else {
                    return false;
                };
                if rel.parent() != pin.parent() {
                    return false;
                }
                let (pin_name, name) = (pin_name.as_encoded_bytes(), name.as_encoded_bytes());
                name.len() > pin_name.len()
                    && name.starts_with(pin_name)
                    && matches!(name[pin_name.len()], b'.' | b'~')
            }
        })
    }

    /// Whether `rel` or a folder above it was deleted locally.
    pub fn is_hidden(&self, rel: &Path) -> bool {
        rel.ancestors().any(|a| self.hidden.contains(a))
    }

    /// The saved attributes of `rel`, if its folder's listing is known.
    /// `Some(None)`: the folder is known and `rel` is not in it.
    pub fn listed(&self, rel: &Path) -> Option<Option<&Listed>> {
        let name = rel.file_name()?;
        let parent = rel.parent().unwrap_or(Path::new(""));
        let items = self.listings.get(parent)?;
        Some(items.iter().find(|l| l.name == name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(pins: &[&str]) -> PinView {
        PinView {
            pins: pins.iter().map(PathBuf::from).collect(),
            ..PinView::default()
        }
    }

    #[test]
    fn coverage() {
        let v = view(&["Passwords/db.kdbx", "Work"]);
        assert!(v.covers(Path::new("Passwords/db.kdbx")));
        assert!(v.covers(Path::new("Passwords/db.kdbx.Ab12Cd")));
        assert!(v.covers(Path::new("Passwords/db.kdbx~")));
        assert!(!v.covers(Path::new("Passwords/db.kdbxold")));
        assert!(!v.covers(Path::new("Passwords/other.kdbx")));
        assert!(!v.covers(Path::new("db.kdbx.tmp")), "same name elsewhere");
        assert!(v.covers(Path::new("Work")));
        assert!(v.covers(Path::new("Work/a/b.txt")));
        assert!(!v.covers(Path::new("Workshop/a.txt")));
        assert!(!v.covers(Path::new("")));
        assert!(view(&[""]).covers(Path::new("anything")));
    }

    #[test]
    fn hidden_and_listed() {
        let mut v = view(&["Work"]);
        v.hidden.insert("Work/old".into());
        assert!(v.is_hidden(Path::new("Work/old/a.txt")));
        assert!(!v.is_hidden(Path::new("Work/new.txt")));
        v.listings.insert(
            "Docs".into(),
            vec![Listed {
                name: "a.txt".into(),
                dir: false,
                size: 3,
                modified: SystemTime::UNIX_EPOCH,
            }],
        );
        assert_eq!(v.listed(Path::new("Docs/a.txt")).unwrap().unwrap().size, 3);
        assert_eq!(v.listed(Path::new("Docs/b.txt")), Some(None));
        assert_eq!(v.listed(Path::new("Other/a.txt")), None);
    }
}
