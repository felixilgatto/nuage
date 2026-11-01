//! Microsoft OneDrive provider (the personal OneDrive of the signed-in
//! account). User documentation: docs/PROVIDERS.md.

use nuage_core::{AuthSpec, Capabilities, CustomClient, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

#[derive(Debug, Clone, Copy, Default)]
pub struct OneDrive;

impl Provider for OneDrive {
    fn id(&self) -> &'static str {
        "onedrive"
    }

    fn display_name(&self) -> &str {
        "OneDrive"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "onedrive"
    }

    fn auth(&self) -> AuthSpec {
        // rclone's own Microsoft client works for personal accounts; an own
        // client (Azure app registration) is possible, never required.
        AuthSpec::OAuthBrowser {
            custom_client: CustomClient::Optional,
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        let mut params = Vec::new();
        if let Some(id) = &input.client_id {
            params.push(("client_id".to_owned(), id.clone()));
        }
        if let Some(secret) = &input.client_secret {
            params.push(("client_secret".to_owned(), secret.clone()));
        }
        if let Some(token) = &input.token {
            params.push(("token".to_owned(), token.clone()));
        }
        // Answers to rclone's questions after the sign-in: the account's
        // own OneDrive (not SharePoint), and yes, that drive.
        params.push(("config_type".to_owned(), "onedrive".to_owned()));
        params.push(("config_drive_ok".to_owned(), "true".to_owned()));
        params
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: true,
            server_side_copy: true,
            server_side_move: true,
            // Deleted items go to the OneDrive recycle bin.
            trash: true,
            empty_trash: false,
            native_docs: false,
            hashes: vec!["quickxor".into(), "sha1".into()],
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions {
            // OneDrive reports changes (delta): one minute, like Drive.
            poll_interval_secs: Some(60),
            ..VfsOptions::default()
        }
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        // Office documents are regular files on OneDrive.
        None
    }

    fn client_id_suffix(&self) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_params() {
        let input = SetupInput {
            name: "OneDrive".into(),
            ..SetupInput::default()
        };
        let params = OneDrive.config_params(&input);
        assert!(params.contains(&("config_type".into(), "onedrive".into())));
        assert!(params.contains(&("config_drive_ok".into(), "true".into())));
        assert!(
            !params.iter().any(|(k, _)| k == "client_id"),
            "rclone's client"
        );

        let input = SetupInput {
            client_id: Some("app".into()),
            client_secret: Some("secret".into()),
            ..input
        };
        let params = OneDrive.config_params(&input);
        assert!(params.contains(&("client_id".into(), "app".into())));
    }

    #[test]
    fn info() {
        let info = OneDrive.info();
        assert_eq!(info.oauth_client, Some(CustomClient::Optional));
        assert!(info.client_guide.is_empty());
        assert!(info.capabilities.trash);
    }
}
