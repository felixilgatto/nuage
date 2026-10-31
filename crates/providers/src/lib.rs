//! Storage providers.
//!
//! rclone does the I/O for every backend, so a provider is mostly
//! declarative: rclone backend type, authentication flow, configuration
//! parameters, capabilities and recommended VFS options. Each provider lives
//! in its own module behind its own Cargo feature; the [`ProviderRegistry`]
//! contains the enabled ones.
//!
//! Adding a provider means adding a module here, a feature in `Cargo.toml`
//! and a line in [`ProviderRegistry::with_enabled_providers`] — without
//! touching `engine`, `ui`, `tray` or `cli`.

#[cfg(feature = "gdrive")]
pub mod gdrive;
#[cfg(feature = "local")]
pub mod local;

mod registry;

pub use registry::ProviderRegistry;

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, ProviderInfo, SetupInput, VfsOptions};

/// A storage provider (Google Drive, OneDrive, local…).
pub trait Provider: Send + Sync {
    /// Stable identifier, e.g. `"gdrive"`.
    fn id(&self) -> &'static str;
    /// Name shown to the user, e.g. `"Google Drive"`.
    fn display_name(&self) -> &str;
    /// Icon name from the icon theme.
    fn icon_name(&self) -> &str;
    /// rclone backend type, e.g. `"drive"`.
    fn rclone_backend(&self) -> &'static str;
    /// How accounts of this provider authenticate.
    fn auth(&self) -> AuthSpec;
    /// Parameters passed to `config/create`, e.g. `("scope", "drive")`.
    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)>;
    /// Capabilities known in advance; completed at runtime with
    /// `operations/fsinfo`.
    fn static_capabilities(&self) -> Capabilities;
    /// VFS options recommended for mounts of this provider.
    fn recommended_vfs_opts(&self) -> VfsOptions;
    /// How provider-native documents (Google Docs…) are exposed, if any.
    fn native_docs(&self) -> Option<NativeDocPolicy>;

    /// Description sent to clients, which do not link this crate.
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.id().to_owned(),
            display_name: self.display_name().to_owned(),
            icon_name: self.icon_name().to_owned(),
            auth: self.auth().kind(),
            capabilities: self.static_capabilities(),
        }
    }
}
