use std::fmt;

use nuage_core::ProviderInfo;

use crate::Provider;

/// The providers compiled into this build, selected by Cargo features.
pub struct ProviderRegistry {
    providers: Vec<Box<dyn Provider>>,
}

impl ProviderRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// A registry containing every provider enabled by Cargo features, in
    /// the order shown to the user.
    pub fn with_enabled_providers() -> Self {
        #[allow(unused_mut)]
        let mut registry = Self::new();
        #[cfg(feature = "gdrive")]
        registry.register(Box::new(crate::gdrive::GoogleDrive));
        #[cfg(feature = "onedrive")]
        registry.register(Box::new(crate::onedrive::OneDrive));
        #[cfg(feature = "dropbox")]
        registry.register(Box::new(crate::dropbox::Dropbox));
        #[cfg(feature = "pcloud")]
        registry.register(Box::new(crate::pcloud::PCloud));
        #[cfg(feature = "protondrive")]
        registry.register(Box::new(crate::protondrive::ProtonDrive));
        #[cfg(feature = "nextcloud")]
        registry.register(Box::new(crate::nextcloud::Nextcloud));
        #[cfg(feature = "webdav")]
        registry.register(Box::new(crate::webdav::WebDav));
        #[cfg(feature = "s3")]
        registry.register(Box::new(crate::s3::S3));
        #[cfg(feature = "local")]
        registry.register(Box::new(crate::local::Local));
        registry
    }

    /// Adds a provider. A provider with the same id replaces the previous one.
    pub fn register(&mut self, provider: Box<dyn Provider>) {
        self.providers.retain(|p| p.id() != provider.id());
        self.providers.push(provider);
    }

    pub fn get(&self, id: &str) -> Option<&dyn Provider> {
        self.providers
            .iter()
            .find(|p| p.id() == id)
            .map(|p| p.as_ref())
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Provider> {
        self.providers.iter().map(|p| p.as_ref())
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    /// Descriptions of all providers, for clients.
    pub fn infos(&self) -> Vec<ProviderInfo> {
        self.iter().map(|p| p.info()).collect()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::with_enabled_providers()
    }
}

impl fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.providers.iter().map(|p| p.id()))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_providers() {
        let registry = ProviderRegistry::with_enabled_providers();
        let features = [
            ("local", cfg!(feature = "local")),
            ("gdrive", cfg!(feature = "gdrive")),
            ("onedrive", cfg!(feature = "onedrive")),
            ("dropbox", cfg!(feature = "dropbox")),
            ("pcloud", cfg!(feature = "pcloud")),
            ("protondrive", cfg!(feature = "protondrive")),
            ("nextcloud", cfg!(feature = "nextcloud")),
            ("webdav", cfg!(feature = "webdav")),
            ("s3", cfg!(feature = "s3")),
        ];
        for (id, enabled) in features {
            assert_eq!(registry.get(id).is_some(), enabled, "{id}");
        }
        assert!(registry.get("nope").is_none());
        let ids: Vec<_> = registry.iter().map(|p| p.id()).collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len());
    }
}
