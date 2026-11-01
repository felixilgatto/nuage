//! Request and answer shapes of the rc endpoints used by Nuage.
//!
//! Field names were checked against rclone v1.75.2. They change between
//! rclone versions: the integration tests of `nuage-engine` fail if one that
//! Nuage relies on disappears.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Answer of `core/version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreVersion {
    /// e.g. `"v1.71.0"`.
    pub version: String,
    /// e.g. `[1, 71, 0]`.
    #[serde(default)]
    pub decomposed: Vec<u32>,
    #[serde(rename = "isGit", default)]
    pub is_git: bool,
    #[serde(rename = "isBeta", default)]
    pub is_beta: bool,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(rename = "goVersion", default)]
    pub go_version: String,
}

/// Answer of `config/create` and `config/update`.
///
/// With `nonInteractive`, rclone stops at the first question it cannot
/// answer by itself and returns it in `State` / `Option`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConfigOutcome {
    /// Empty when the configuration is complete.
    #[serde(rename = "State", default)]
    pub state: String,
    /// Error from the previous answer, if any.
    #[serde(rename = "Error", default)]
    pub error: String,
    /// The pending question, when `state` is not empty.
    #[serde(rename = "Option", default)]
    pub option: Option<serde_json::Value>,
    #[serde(rename = "Result", default)]
    pub result: String,
}

impl ConfigOutcome {
    pub fn is_complete(&self) -> bool {
        self.state.is_empty()
    }

    /// Name of the pending question, e.g. `"config_refresh_token"`.
    pub fn pending_question(&self) -> Option<&str> {
        self.option.as_ref()?.get("Name")?.as_str()
    }
}

/// What [`RcClient::config_continue`](crate::RcClient::config_continue)
/// returns.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigStep {
    Done(ConfigOutcome),
    /// Running as rc job `id`.
    Job(u64),
}

/// `job/status` of a configuration job.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct ConfigJob {
    #[serde(default)]
    pub finished: bool,
    #[serde(default)]
    pub success: bool,
    /// Error of the job, empty if none.
    #[serde(default)]
    pub error: String,
    /// The configuration outcome once finished.
    #[serde(default)]
    pub output: Option<ConfigOutcome>,
}

/// Answer of `config/oauthstatus`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct OAuthStatus {
    /// `"running"` or `"stopped"`.
    pub status: String,
    #[serde(rename = "authUrl", default)]
    pub auth_url: Option<String>,
}

/// Answer of `operations/fsinfo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct FsInfo {
    #[serde(rename = "Name", default)]
    pub name: String,
    #[serde(rename = "Root", default)]
    pub root: String,
    #[serde(rename = "Features", default)]
    pub features: BTreeMap<String, bool>,
    #[serde(rename = "Hashes", default)]
    pub hashes: Vec<String>,
}

/// Answer of `operations/about`. Fields a backend does not report are
/// omitted by rclone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct About {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub free: Option<u64>,
    pub trashed: Option<u64>,
    pub other: Option<u64>,
    pub objects: Option<u64>,
}

/// Options of `mount/mount`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MountRequest {
    pub fs: String,
    #[serde(rename = "mountPoint")]
    pub mount_point: String,
    #[serde(rename = "vfsOpt")]
    pub vfs_opt: VfsOpt,
    #[serde(rename = "mountOpt")]
    pub mount_opt: MountOpt,
    /// Filters of this mount only (`_filter`).
    #[serde(rename = "_filter", skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
}

/// rclone filter options of one call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Filter {
    /// Rules in rclone's syntax, e.g. `"- *.tmp"`.
    #[serde(rename = "FilterRule")]
    pub filter_rule: Vec<String>,
}

/// The subset of rclone's VFS options set by Nuage.
///
/// Durations are rclone duration strings (`"8760h"`), sizes are byte counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct VfsOpt {
    #[serde(rename = "CacheMode")]
    pub cache_mode: String,
    #[serde(rename = "CacheMaxSize", skip_serializing_if = "Option::is_none")]
    pub cache_max_size: Option<u64>,
    #[serde(rename = "CacheMaxAge", skip_serializing_if = "Option::is_none")]
    pub cache_max_age: Option<String>,
    #[serde(rename = "WriteBack", skip_serializing_if = "Option::is_none")]
    pub write_back: Option<String>,
    #[serde(rename = "DirCacheTime", skip_serializing_if = "Option::is_none")]
    pub dir_cache_time: Option<String>,
    #[serde(rename = "PollInterval", skip_serializing_if = "Option::is_none")]
    pub poll_interval: Option<String>,
}

/// The subset of rclone's mount options set by Nuage.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MountOpt {
    /// Name shown by file managers.
    #[serde(rename = "VolumeName", skip_serializing_if = "Option::is_none")]
    pub volume_name: Option<String>,
}

/// One entry of `mount/listmounts`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MountInfo {
    #[serde(rename = "Fs")]
    pub fs: String,
    #[serde(rename = "MountPoint")]
    pub mount_point: String,
}

/// Answer of `vfs/stats`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct VfsStats {
    pub fs: String,
    /// Absent when the VFS has no disk cache (`CacheMode=off`).
    #[serde(rename = "diskCache")]
    pub disk_cache: Option<DiskCache>,
}

/// `diskCache` of `vfs/stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct DiskCache {
    /// Directory of the cached data.
    #[serde(default)]
    pub path: String,
    /// Directory of the cache metadata.
    #[serde(rename = "pathMeta", default)]
    pub path_meta: String,
    /// Refreshed only on the cache poll (every minute by default).
    #[serde(rename = "bytesUsed")]
    pub bytes_used: u64,
    #[serde(rename = "erroredFiles")]
    pub errored_files: u32,
    pub files: u64,
    #[serde(rename = "outOfSpace")]
    pub out_of_space: bool,
    #[serde(rename = "uploadsInProgress")]
    pub uploads_in_progress: u32,
    #[serde(rename = "uploadsQueued")]
    pub uploads_queued: u32,
}

/// One item of `vfs/queue`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct QueueItem {
    pub name: String,
    pub id: u64,
    pub size: i64,
    /// Seconds until rclone uploads the item on its own; negative once due.
    pub expiry: f64,
    /// Write-back delay the item was queued with, in seconds.
    pub delay: f64,
    pub tries: u32,
    pub uploading: bool,
}

impl QueueItem {
    /// Seconds since the item was (re)queued.
    pub fn age_secs(&self) -> f64 {
        self.delay - self.expiry
    }
}
