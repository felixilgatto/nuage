//! Data types used by the `Provider` trait (defined in `nuage-providers`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// How an account of a provider is authenticated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSpec {
    /// OAuth in a web browser, driven by rclone. On a headless machine the
    /// user can also pass a token generated elsewhere (`rclone authorize`).
    OAuthBrowser {
        /// Whether the user supplies their own OAuth client ID and secret.
        custom_client: CustomClient,
    },
    /// A form of credentials (user name, password, URL…).
    Credentials { fields: Vec<CredentialField> },
    /// No authentication (e.g. the `local` provider).
    None,
}

impl AuthSpec {
    pub fn kind(&self) -> AuthKind {
        match self {
            AuthSpec::OAuthBrowser { .. } => AuthKind::OAuthBrowser,
            AuthSpec::Credentials { .. } => AuthKind::Credentials,
            AuthSpec::None => AuthKind::None,
        }
    }
}

/// Whether an OAuth provider uses a client created by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomClient {
    /// rclone's built-in client only.
    Unsupported,
    /// rclone's built-in client by default, the user's own client as an option.
    Optional,
    /// The user must create and provide their own client (guided setup).
    Required,
}

/// [`AuthSpec`] without its details, for listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    OAuthBrowser,
    Credentials,
    None,
}

/// One field of a credentials form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialField {
    /// rclone configuration key.
    pub key: String,
    /// Label shown to the user (English, translated by the UI).
    pub label: String,
    /// The value is a secret and must never be logged or displayed.
    pub secret: bool,
    pub required: bool,
    /// The value is a folder of this computer (the interface offers a
    /// folder chooser).
    #[serde(default)]
    pub folder: bool,
}

/// One step of the guide to create an OAuth client, with the page where it
/// is done.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuideStep {
    /// What to do (English, translated by the interface).
    pub title: String,
    pub url: String,
}

/// What the user provided when adding an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupInput {
    /// Account name chosen by the user (also the default folder name).
    pub name: String,
    /// OAuth token JSON generated elsewhere (headless setup).
    pub token: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    /// Values of an [`AuthSpec::Credentials`] form, by key.
    pub credentials: BTreeMap<String, String>,
}

/// rclone VFS cache mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    Off,
    Minimal,
    Writes,
    Full,
}

/// VFS options recommended by a provider for its mounts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VfsOptions {
    pub cache_mode: CacheMode,
    /// Maximum cache size in bytes (`CacheMaxSize`).
    pub cache_max_size: Option<u64>,
    /// Maximum age of cached files, in seconds (`CacheMaxAge`).
    pub cache_max_age_secs: Option<u64>,
    /// How long directory listings are cached, in seconds (`DirCacheTime`).
    pub dir_cache_time_secs: Option<u64>,
    /// Interval of the backend change polling, in seconds (`PollInterval`).
    pub poll_interval_secs: Option<u64>,
}

impl Default for VfsOptions {
    fn default() -> Self {
        Self {
            cache_mode: CacheMode::Full,
            cache_max_size: Some(crate::DEFAULT_CACHE_LIMIT),
            cache_max_age_secs: Some(30 * 24 * 3600),
            dir_cache_time_secs: None,
            poll_interval_secs: None,
        }
    }
}

/// How provider-native documents (Google Docs…) are exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeDocPolicy {
    /// Exported as web shortcuts that open in the browser, using rclone's
    /// export formats (e.g. `"link.html"` or `"desktop"`).
    WebShortcut { export_formats: String },
}
