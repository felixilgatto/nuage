use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// What a provider (or a connected account) can do.
///
/// Interfaces hide or refuse actions according to these flags, never
/// according to the provider name. Static capabilities declared by a provider
/// are completed at runtime with the backend features reported by rclone's
/// `operations/fsinfo` (see [`Capabilities::merge_fsinfo_features`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Quota reporting (`operations/about`).
    pub quota: bool,
    /// Shareable links (`operations/publiclink`).
    pub public_link: bool,
    /// Server-side copy.
    pub server_side_copy: bool,
    /// Server-side move / rename.
    pub server_side_move: bool,
    /// Deleted items go to a trash.
    pub trash: bool,
    /// The trash can be emptied.
    pub empty_trash: bool,
    /// Provider-native documents exposed as web shortcuts.
    pub native_docs: bool,
    /// Hash algorithms supported by the backend (e.g. `"md5"`, `"sha1"`).
    pub hashes: Vec<String>,
}

impl Capabilities {
    /// Merges the `Features` map returned by rclone's `operations/fsinfo`.
    ///
    /// A feature reported as present enables the matching capability; a
    /// feature reported as absent disables it. Unknown features are ignored.
    pub fn merge_fsinfo_features(&mut self, features: &BTreeMap<String, bool>) {
        let set = |flag: &mut bool, key: &str| {
            if let Some(&value) = features.get(key) {
                *flag = value;
            }
        };
        set(&mut self.quota, "About");
        set(&mut self.public_link, "PublicLink");
        set(&mut self.server_side_copy, "Copy");
        set(&mut self.server_side_move, "Move");
        set(&mut self.empty_trash, "CleanUp");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_fsinfo() {
        let mut caps = Capabilities {
            public_link: true,
            ..Capabilities::default()
        };
        let features = BTreeMap::from([
            ("About".to_owned(), true),
            ("PublicLink".to_owned(), false),
            ("SomethingNew".to_owned(), true),
        ]);
        caps.merge_fsinfo_features(&features);
        assert!(caps.quota);
        assert!(!caps.public_link);
        assert!(!caps.server_side_copy);
    }
}
