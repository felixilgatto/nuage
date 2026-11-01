//! `local` provider: a folder of the local disk, mounted elsewhere through
//! rclone. Needs no account; used by tests and CI.
//!
//! The remote is an rclone `alias` of the folder, so that, as for every
//! other provider, the account root is simply `<remote>:`.

use std::path::Path;

use nuage_core::{
    AuthSpec, Capabilities, CredentialField, NativeDocPolicy, SetupInput, VfsOptions,
};

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
        "alias"
    }

    fn auth(&self) -> AuthSpec {
        AuthSpec::Credentials {
            fields: vec![CredentialField {
                key: ROOT_KEY.to_owned(),
                label: "Folder".to_owned(),
                secret: false,
                required: true,
            }],
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        input
            .credentials
            .get(ROOT_KEY)
            .map(|root| vec![("remote".to_owned(), root.clone())])
            .unwrap_or_default()
    }

    fn validate(&self, input: &SetupInput) -> Result<(), String> {
        let root = input.credentials.get(ROOT_KEY).ok_or_else(|| {
            format!("Choose the folder to mount with --param {ROOT_KEY}=<folder>.")
        })?;
        let path = Path::new(root);
        if !path.is_absolute() {
            return Err(format!("The folder must be an absolute path, not {root}."));
        }
        if !path.is_dir() {
            return Err(format!("{root} does not exist or is not a folder."));
        }
        Ok(())
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
        assert!(Local.config_params(&SetupInput::default()).is_empty());
        assert_eq!(Local.info().id, "local");
        let dir = std::env::temp_dir();
        let input = SetupInput {
            credentials: [(ROOT_KEY.to_owned(), dir.display().to_string())].into(),
            ..SetupInput::default()
        };
        assert_eq!(
            Local.config_params(&input),
            vec![("remote".to_owned(), dir.display().to_string())]
        );
        assert!(Local.validate(&input).is_ok());
        assert!(Local.validate(&SetupInput::default()).is_err());
        let relative = SetupInput {
            credentials: [(ROOT_KEY.to_owned(), "relative".to_owned())].into(),
            ..SetupInput::default()
        };
        assert!(Local.validate(&relative).is_err());
    }
}
