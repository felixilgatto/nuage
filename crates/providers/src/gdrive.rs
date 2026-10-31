//! Google Drive provider ("My Drive" only in v1).
//!
//! **Declarative stub**: the values below follow the project brief; the
//! OAuth flows are completed in milestone 4.

use nuage_core::{AuthSpec, Capabilities, CustomClient, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

#[derive(Debug, Clone, Copy, Default)]
pub struct GoogleDrive;

impl Provider for GoogleDrive {
    fn id(&self) -> &'static str {
        "gdrive"
    }

    fn display_name(&self) -> &str {
        "Google Drive"
    }

    fn icon_name(&self) -> &str {
        // TODO(milestone 4): ship a provider icon with the application.
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "drive"
    }

    fn auth(&self) -> AuthSpec {
        // rclone's shared client is being retired and a Nuage-owned client
        // would need Google verification, so each user creates their own
        // "Desktop app" client, with a guided setup.
        AuthSpec::OAuthBrowser {
            custom_client: CustomClient::Required,
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        // `drive` scope: files only, no other Google service.
        let mut params = vec![("scope".to_owned(), "drive".to_owned())];
        if let Some(id) = &input.client_id {
            params.push(("client_id".to_owned(), id.clone()));
        }
        if let Some(secret) = &input.client_secret {
            params.push(("client_secret".to_owned(), secret.clone()));
        }
        if let Some(token) = &input.token {
            params.push(("token".to_owned(), token.clone()));
        }
        params
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: true,
            server_side_copy: true,
            server_side_move: true,
            trash: true,
            empty_trash: true,
            native_docs: true,
            hashes: vec!["md5".into(), "sha1".into(), "sha256".into()],
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions {
            // Drive supports change polling; one minute is rclone's default.
            poll_interval_secs: Some(60),
            ..VfsOptions::default()
        }
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        // `link.html` opens in the default browser from Nautilus; `desktop`
        // opens in a text editor. The link files are read-only.
        Some(NativeDocPolicy::WebShortcut {
            export_formats: "link.html".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_params() {
        let input = SetupInput {
            client_id: Some("id".into()),
            client_secret: Some("secret".into()),
            ..SetupInput::default()
        };
        let params = GoogleDrive.config_params(&input);
        assert_eq!(params[0], ("scope".to_owned(), "drive".to_owned()));
        assert!(params.contains(&("client_id".to_owned(), "id".to_owned())));
        assert_eq!(GoogleDrive.rclone_backend(), "drive");
    }
}
