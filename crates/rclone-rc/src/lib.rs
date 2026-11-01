//! Typed client for the rclone remote control API (`rclone rcd`) and
//! supervision of the `rclone rcd` process.
//!
//! Only the endpoints used by Nuage are typed (see `docs/ARCHITECTURE.md`).
//! Field names change between rclone versions: they are checked against the
//! bundled rclone by the integration tests of `nuage-engine`.

mod client;
mod supervisor;
mod types;

pub use client::{RcClient, RcError};
pub use supervisor::{RcdConfig, RcdSupervisor, cache_deleted};
pub use types::{
    About, ConfigJob, ConfigOutcome, ConfigStep, CoreVersion, DiskCache, Filter, FsInfo, JobStatus,
    ListItem, MountInfo, MountOpt, MountRequest, OAuthStatus, QueueItem, VfsOpt, VfsStats,
};
