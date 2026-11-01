//! Nuage domain model.
//!
//! This crate has no dependency on any other Nuage crate and no GTK
//! dependency. It defines the vocabulary shared by the daemon, the engine,
//! the providers and every client: accounts, remote entries, quotas, sync
//! states, capabilities, and the [`Command`] / [`Event`] messages exchanged
//! over IPC.

mod account;
mod capabilities;
mod command;
mod entry;
mod error;
mod event;
mod exclusion;
mod pin;
mod provider;
mod quota;
mod state;

pub use account::{Account, AccountId, DEFAULT_CACHE_LIMIT, RemotePath};
pub use capabilities::Capabilities;
pub use command::{
    AccountAddArgs, AccountStatus, Command, ConfigEntry, DaemonInfo, ProviderInfo, Reply, SignIn,
    StatusReport,
};
pub use entry::{Entry, EntryKind, NativeDoc, NativeDocKind};
pub use error::{Error, Result};
pub use event::Event;
pub use exclusion::{DEFAULT_PATTERNS, Exclusion, ExclusionRule, ExclusionSet};
pub use pin::{Pin, PinKind, PinState};
pub use provider::{
    AuthKind, AuthSpec, CacheMode, CredentialField, CustomClient, GuideStep, NativeDocPolicy,
    SetupInput, VfsOptions,
};
pub use quota::Quota;
pub use state::SyncState;

/// Flatpak application identifier.
pub const APP_ID: &str = "io.github.felixilgatto.Nuage";

/// Version of the Nuage crates (shared by the daemon and its clients).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
