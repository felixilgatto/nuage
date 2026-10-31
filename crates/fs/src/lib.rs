//! `nuage-fs`: overlay FUSE filesystem.
//!
//! **Placeholder.** Mounted at the user's folder on top of a hidden rclone
//! mount, it relays to rclone and enforces exclusions locally (a locally
//! created file matching an exclusion is kept local-only and never reaches
//! rclone); offline pinning is added in v1.1. See `docs/ARCHITECTURE.md`.
//! Nothing depends on this crate yet.
