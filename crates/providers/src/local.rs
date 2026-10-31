//! `local` provider: a folder of the local disk seen through rclone's
//! `local` backend. Needs no credentials; used by tests and CI.

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

/// Key of [`SetupInput::credentials`] holding the root folder of the remote.
pub const ROOT_KEY: &str = "root";

#[derive(Debug, Clone, Copy, Default)]
pub struct Local;

impl Provider for Local {
    fn id(&self) -> &'static str {
        "local"
    }

    fn display_name(&self) -> &str {
        "Local folder"
    }

    fn icon_name(&self) -> &str {
        "folder-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "local"
    }

    fn auth(&self) -> AuthSpec {
        AuthSpec::None
    }

    fn config_params(&self, _input: &SetupInput) -> Vec<(String, String)> {
        // The `local` backend has no required parameter; the root folder is
        // part of the remote path (`<remote>:<root>`), see `ROOT_KEY`.
        Vec::new()
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: false,
            server_side_copy: false,
            server_side_move: true,
            trash: false,
            empty_trash: false,
            native_docs: false,
            hashes: vec!["md5".into(), "sha1".into()],
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions::default()
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declarative() {
        assert_eq!(Local.auth(), AuthSpec::None);
        assert!(Local.config_params(&SetupInput::default()).is_empty());
        assert_eq!(Local.info().id, "local");
    }
}
