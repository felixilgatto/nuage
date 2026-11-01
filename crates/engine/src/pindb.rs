//! Pin metadata (SQLite, `data/pins.db`): the pins, the reference state of
//! each pinned file (both sides at the last sync), and saved listings for
//! browsing offline.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use nuage_core::{AccountId, Error, PinKind, Result};

/// Remote side of a file at the last sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteSig {
    pub size: i64,
    pub mtime: String,
    pub hash: Option<String>,
}

impl RemoteSig {
    /// Same content: by hash when both have one, else size and date.
    pub fn same(&self, other: &RemoteSig) -> bool {
        match (&self.hash, &other.hash) {
            (Some(a), Some(b)) => a == b,
            _ => self.size == other.size && self.mtime == other.mtime,
        }
    }
}

/// Local side (the store copy) at the last sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LocalSig {
    pub size: u64,
    pub mtime_ns: i128,
}

/// The reference state of a pinned item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reference {
    pub dir: bool,
    pub remote: Option<RemoteSig>,
    pub local: Option<LocalSig>,
}

/// An item of a saved listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedItem {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// Seconds since the epoch.
    pub mtime: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PinRow {
    pub account: AccountId,
    pub path: String,
    pub kind: PinKind,
    pub pinned_at: chrono::DateTime<chrono::Utc>,
}

fn db_error(e: rusqlite::Error) -> Error {
    Error::Internal(format!("pin database: {e}"))
}

fn kind_text(kind: PinKind) -> &'static str {
    match kind {
        PinKind::File => "file",
        PinKind::FolderRecursive => "folder",
    }
}

/// Whether `path` is `dir` or inside it (`dir` empty: the root).
pub(crate) fn within(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

pub(crate) struct PinDb {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for PinDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinDb").finish_non_exhaustive()
    }
}

impl PinDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).map_err(db_error)?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(db_error)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS pins (
                 account TEXT NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL,
                 pinned_at TEXT NOT NULL, PRIMARY KEY (account, path));
             CREATE TABLE IF NOT EXISTS files (
                 account TEXT NOT NULL, path TEXT NOT NULL, dir INTEGER NOT NULL,
                 remote_size INTEGER, remote_mtime TEXT, remote_hash TEXT,
                 local_size INTEGER, local_mtime_ns TEXT,
                 PRIMARY KEY (account, path));
             CREATE TABLE IF NOT EXISTS listings (
                 account TEXT NOT NULL, dir TEXT NOT NULL, entries TEXT NOT NULL,
                 PRIMARY KEY (account, dir));",
        )
        .map_err(db_error)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("pin database lock")
    }

    // Pins

    pub fn pins(&self, account: Option<&AccountId>) -> Result<Vec<PinRow>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT account, path, kind, pinned_at FROM pins
                 WHERE ?1 IS NULL OR account = ?1 ORDER BY account, path",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![account.map(AccountId::as_str)], |r| {
                let kind: String = r.get(2)?;
                let at: String = r.get(3)?;
                Ok(PinRow {
                    account: AccountId::new(r.get::<_, String>(0)?),
                    path: r.get(1)?,
                    kind: if kind == "folder" {
                        PinKind::FolderRecursive
                    } else {
                        PinKind::File
                    },
                    pinned_at: chrono::DateTime::parse_from_rfc3339(&at)
                        .map(|t| t.with_timezone(&chrono::Utc))
                        .unwrap_or_default(),
                })
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_error)
    }

    pub fn add_pin(&self, account: &AccountId, path: &str, kind: PinKind) -> Result<()> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO pins (account, path, kind, pinned_at) VALUES (?1, ?2, ?3, ?4)",
                params![
                    account.as_str(),
                    path,
                    kind_text(kind),
                    chrono::Utc::now().to_rfc3339()
                ],
            )
            .map(drop)
            .map_err(db_error)
    }

    /// Removes a pin and the references under it.
    pub fn remove_pin(&self, account: &AccountId, path: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM pins WHERE account = ?1 AND path = ?2",
            params![account.as_str(), path],
        )
        .map_err(db_error)?;
        let refs: Vec<String> = Self::paths_under(&conn, account, path)?;
        for p in refs {
            conn.execute(
                "DELETE FROM files WHERE account = ?1 AND path = ?2",
                params![account.as_str(), p],
            )
            .map_err(db_error)?;
        }
        Ok(())
    }

    /// Forgets everything about an account.
    pub fn remove_account(&self, account: &AccountId) -> Result<()> {
        let conn = self.conn();
        for table in ["pins", "files", "listings"] {
            conn.execute(
                &format!("DELETE FROM {table} WHERE account = ?1"),
                params![account.as_str()],
            )
            .map_err(db_error)?;
        }
        Ok(())
    }

    // References

    fn paths_under(conn: &Connection, account: &AccountId, dir: &str) -> Result<Vec<String>> {
        let mut stmt = conn
            .prepare("SELECT path FROM files WHERE account = ?1")
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![account.as_str()], |r| r.get::<_, String>(0))
            .map_err(db_error)?;
        Ok(rows
            .filter_map(std::result::Result::ok)
            .filter(|p| within(p, dir))
            .collect())
    }

    pub fn references(&self, account: &AccountId) -> Result<HashMap<String, Reference>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare(
                "SELECT path, dir, remote_size, remote_mtime, remote_hash, local_size, local_mtime_ns
                 FROM files WHERE account = ?1",
            )
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![account.as_str()], |r| {
                let remote_size: Option<i64> = r.get(2)?;
                let remote_mtime: Option<String> = r.get(3)?;
                let local_size: Option<i64> = r.get(5)?;
                let local_mtime: Option<String> = r.get(6)?;
                Ok((
                    r.get::<_, String>(0)?,
                    Reference {
                        dir: r.get::<_, i64>(1)? != 0,
                        remote: remote_size
                            .zip(remote_mtime)
                            .map(|(size, mtime)| RemoteSig {
                                size,
                                mtime,
                                hash: r.get(4).ok().flatten(),
                            }),
                        local: local_size.zip(local_mtime).and_then(|(size, mtime)| {
                            Some(LocalSig {
                                size: size as u64,
                                mtime_ns: mtime.parse().ok()?,
                            })
                        }),
                    },
                ))
            })
            .map_err(db_error)?;
        rows.collect::<rusqlite::Result<_>>().map_err(db_error)
    }

    pub fn set_reference(&self, account: &AccountId, path: &str, r: &Reference) -> Result<()> {
        self.conn()
            .execute(
                "INSERT OR REPLACE INTO files
                 (account, path, dir, remote_size, remote_mtime, remote_hash, local_size, local_mtime_ns)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    account.as_str(),
                    path,
                    i64::from(r.dir),
                    r.remote.as_ref().map(|s| s.size),
                    r.remote.as_ref().map(|s| s.mtime.clone()),
                    r.remote.as_ref().and_then(|s| s.hash.clone()),
                    r.local.map(|s| s.size as i64),
                    r.local.map(|s| s.mtime_ns.to_string()),
                ],
            )
            .map(drop)
            .map_err(db_error)
    }

    pub fn remove_reference(&self, account: &AccountId, path: &str) -> Result<()> {
        self.conn()
            .execute(
                "DELETE FROM files WHERE account = ?1 AND path = ?2",
                params![account.as_str(), path],
            )
            .map(drop)
            .map_err(db_error)
    }

    // Listings

    pub fn save_listing(&self, account: &AccountId, dir: &str, items: &[SavedItem]) -> Result<()> {
        let entries = serde_json::to_string(items).map_err(|e| Error::Internal(e.to_string()))?;
        let conn = self.conn();
        let current: Option<String> = conn
            .query_row(
                "SELECT entries FROM listings WHERE account = ?1 AND dir = ?2",
                params![account.as_str(), dir],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if current.as_deref() == Some(entries.as_str()) {
            return Ok(());
        }
        conn.execute(
            "INSERT OR REPLACE INTO listings (account, dir, entries) VALUES (?1, ?2, ?3)",
            params![account.as_str(), dir, entries],
        )
        .map(drop)
        .map_err(db_error)
    }

    pub fn listings(&self, account: &AccountId) -> Result<HashMap<String, Vec<SavedItem>>> {
        let conn = self.conn();
        let mut stmt = conn
            .prepare("SELECT dir, entries FROM listings WHERE account = ?1")
            .map_err(db_error)?;
        let rows = stmt
            .query_map(params![account.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        Ok(rows
            .filter_map(std::result::Result::ok)
            .filter_map(|(dir, json)| Some((dir, serde_json::from_str(&json).ok()?)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_references_listings() {
        let db = PinDb::in_memory().unwrap();
        let a = AccountId::new("a");
        db.add_pin(&a, "Work", PinKind::FolderRecursive).unwrap();
        db.add_pin(&a, "notes.txt", PinKind::File).unwrap();
        db.add_pin(&AccountId::new("b"), "x", PinKind::File)
            .unwrap();
        let pins = db.pins(Some(&a)).unwrap();
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[1].path, "notes.txt");
        assert_eq!(db.pins(None).unwrap().len(), 3);

        let r = Reference {
            dir: false,
            remote: Some(RemoteSig {
                size: 3,
                mtime: "2026-01-01T00:00:00Z".into(),
                hash: Some("md5:abc".into()),
            }),
            local: Some(LocalSig {
                size: 3,
                mtime_ns: 1_700_000_000_123_456_789,
            }),
        };
        db.set_reference(&a, "Work/a.txt", &r).unwrap();
        db.set_reference(&a, "Workshop/b.txt", &r).unwrap();
        assert_eq!(db.references(&a).unwrap()["Work/a.txt"], r);
        db.remove_pin(&a, "Work").unwrap();
        let refs = db.references(&a).unwrap();
        assert!(!refs.contains_key("Work/a.txt"));
        assert!(refs.contains_key("Workshop/b.txt"), "not inside Work");

        let items = vec![SavedItem {
            name: "a.txt".into(),
            dir: false,
            size: 3,
            mtime: 1,
        }];
        db.save_listing(&a, "", &items).unwrap();
        db.save_listing(&a, "", &items).unwrap();
        assert_eq!(db.listings(&a).unwrap()[""], items);
        db.remove_account(&a).unwrap();
        assert!(db.pins(Some(&a)).unwrap().is_empty());
    }

    #[test]
    fn same_content() {
        let sig = |hash: Option<&str>, size, mtime: &str| RemoteSig {
            size,
            mtime: mtime.into(),
            hash: hash.map(str::to_owned),
        };
        assert!(sig(Some("x"), 1, "a").same(&sig(Some("x"), 2, "b")));
        assert!(!sig(Some("x"), 1, "a").same(&sig(Some("y"), 1, "a")));
        assert!(sig(None, 1, "a").same(&sig(Some("y"), 1, "a")));
        assert!(!sig(None, 1, "a").same(&sig(None, 1, "b")));
    }
}
