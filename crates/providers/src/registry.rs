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
        assert_eq!(registry.get("local").is_some(), cfg!(feature = "local"));
        assert_eq!(registry.get("gdrive").is_some(), cfg!(feature = "gdrive"));
        assert!(registry.get("nope").is_none());
        let ids: Vec<_> = registry.iter().map(|p| p.id()).collect();
        let mut unique = ids.clone();
        unique.dedup();
        assert_eq!(ids, unique);
    }
}
