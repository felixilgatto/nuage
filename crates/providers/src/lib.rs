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

#[cfg(feature = "dropbox")]
pub mod dropbox;
#[cfg(feature = "gdrive")]
pub mod gdrive;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "nextcloud")]
pub mod nextcloud;
#[cfg(feature = "onedrive")]
pub mod onedrive;
#[cfg(feature = "pcloud")]
pub mod pcloud;
#[cfg(feature = "protondrive")]
pub mod protondrive;
#[cfg(feature = "s3")]
pub mod s3;
#[cfg(feature = "webdav")]
pub mod webdav;

mod registry;

pub use registry::ProviderRegistry;

use nuage_core::{
    AuthSpec, Capabilities, CredentialField, GuideStep, NativeDocPolicy, ProviderInfo, SetupInput,
    VfsOptions,
};

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

    /// Steps to create the user's own OAuth client, for providers that
    /// need one.
    fn client_guide(&self) -> Vec<GuideStep> {
        Vec::new()
    }

    /// The end of a valid client ID, to check what the user pastes.
    fn client_id_suffix(&self) -> Option<&'static str> {
        None
    }

    /// Other drives the accounts give access to, each becoming an account
    /// of its own (e.g. Google shared drives).
    fn sub_drives(&self) -> Option<nuage_core::SubDrives> {
        None
    }

    /// Order in which to try the choices rclone offers for a question that
    /// [`config_params`](Self::config_params) does not answer (e.g. which
    /// OneDrive drive): lower first, then rclone's order. `choice` is
    /// rclone's description of it.
    fn choice_rank(&self, _question: &str, _choice: &str) -> u8 {
        0
    }

    /// Folder of the remote the account shows (e.g. an S3 bucket),
    /// slash-separated, without leading or trailing slash. Empty: the whole
    /// remote.
    fn root(&self, _input: &SetupInput) -> String {
        String::new()
    }

    /// Checks what the user provided before anything is created. The error
    /// is shown to the user and says what to fix.
    fn validate(&self, _input: &SetupInput) -> Result<(), String> {
        Ok(())
    }

    /// Description sent to clients, which do not link this crate.
    fn info(&self) -> ProviderInfo {
        let auth = self.auth();
        ProviderInfo {
            id: self.id().to_owned(),
            display_name: self.display_name().to_owned(),
            icon_name: self.icon_name().to_owned(),
            auth: auth.kind(),
            oauth_client: match &auth {
                AuthSpec::OAuthBrowser { custom_client } => Some(*custom_client),
                _ => None,
            },
            credentials: match auth {
                AuthSpec::Credentials { fields } => fields,
                _ => Vec::new(),
            },
            client_guide: self.client_guide(),
            client_id_suffix: self.client_id_suffix().map(str::to_owned),
            sub_drives: self.sub_drives(),
            capabilities: self.static_capabilities(),
        }
    }
}

/// The `config/create` parameters of an OAuth provider using rclone's
/// client by default: the user's own client and a token made elsewhere,
/// when given.
#[allow(dead_code)]
pub(crate) fn oauth_params(input: &SetupInput) -> Vec<(String, String)> {
    [
        ("client_id", &input.client_id),
        ("client_secret", &input.client_secret),
        ("token", &input.token),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.as_ref().map(|v| (key.to_owned(), v.clone())))
    .collect()
}

/// A field of a credentials form (not a folder).
#[allow(dead_code)]
pub(crate) fn field(key: &str, label: &str, secret: bool, required: bool) -> CredentialField {
    CredentialField {
        key: key.to_owned(),
        label: label.to_owned(),
        secret,
        required,
        folder: false,
    }
}

/// A value of the credentials form, trimmed; `None` when absent or blank.
#[allow(dead_code)]
pub(crate) fn credential<'a>(input: &'a SetupInput, key: &str) -> Option<&'a str> {
    input
        .credentials
        .get(key)
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
}

/// Checks that the user entered a web address with its `https://`.
#[allow(dead_code)]
pub(crate) fn check_web_address(address: &str) -> Result<(), String> {
    let rest = address
        .strip_prefix("https://")
        .or_else(|| address.strip_prefix("http://"));
    match rest {
        Some(host) if !host.is_empty() && !address.contains(char::is_whitespace) => Ok(()),
        _ => Err(format!(
            "\"{address}\" is not a web address: it starts with https://, e.g. https://cloud.example.com."
        )),
    }
}
