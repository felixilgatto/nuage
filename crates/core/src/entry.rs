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
