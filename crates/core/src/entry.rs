use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A file or folder on a remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    /// Path relative to the account root.
    pub path: String,
    pub kind: EntryKind,
    /// Size in bytes; `None` for folders or when unknown.
    pub size: Option<u64>,
    pub modified: Option<DateTime<Utc>>,
    pub mime: Option<String>,
    /// A content hash, when the backend provides one (`"<algo>:<hex>"`).
    pub hash: Option<String>,
    /// Set for provider-native documents (e.g. Google Docs) that only open in
    /// a web browser.
    pub native_doc: Option<NativeDoc>,
    /// What the browser shows next to the name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<Badge>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Folder
    }

    /// The name to show: a native document without its shortcut suffix.
    pub fn display_name(&self) -> &str {
        self.native_doc.as_ref().map_or(&self.name, |d| &d.title)
    }

    pub fn has(&self, badge: Badge) -> bool {
        self.badges.contains(&badge)
    }
}

/// State of an entry, shown as a badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    /// Waiting to be uploaded, or uploading (for a folder: something in it).
    Uploading,
    /// Excluded from the sync: not in the mounted folder.
    NotSynced,
    /// Listed from the cache while offline, and its content is not cached:
    /// it cannot be opened until the connection is back.
    OnlineOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
}

/// A provider-native document, exposed as a web shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeDoc {
    /// The document's name, without the shortcut suffix.
    pub title: String,
    pub kind: NativeDocKind,
    /// Link that opens the document in a web browser, when known.
    pub web_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeDocKind {
    Document,
    Spreadsheet,
    Presentation,
    Drawing,
    Form,
    Other,
}
