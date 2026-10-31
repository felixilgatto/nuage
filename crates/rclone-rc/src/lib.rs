//! Typed client for the rclone remote control API (`rclone rcd`) and
//! supervision of the `rclone rcd` process.
//!
//! Only `core/version` is implemented so far; the other endpoints listed in
//! `docs/ARCHITECTURE.md` are added with the engine (milestone 3). Field
//! names change between rclone versions: they must be checked against the
//! bundled rclone and covered by integration tests.

mod client;
mod supervisor;

pub use client::{CoreVersion, RcClient, RcError};
pub use supervisor::{RcdConfig, RcdSupervisor};
