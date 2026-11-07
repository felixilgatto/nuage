//! Dropbox provider (the whole Dropbox of the signed-in account). User
//! documentation: docs/PROVIDERS.md.

use nuage_core::{AuthSpec, Capabilities, CustomClient, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

#[derive(Debug, Clone, Copy, Default)]
pub struct Dropbox;

impl Provider for Dropbox {
    fn id(&self) -> &'static str {
        "dropbox"
    }

    fn display_name(&self) -> &str {
        "Dropbox"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "dropbox"
    }

    fn auth(&self) -> AuthSpec {
        // rclone's own Dropbox app works; an own app is possible, never
        // required.
        AuthSpec::OAuthBrowser {
            custom_client: CustomClient::Optional,
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        crate::oauth_params(input)
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: true,
            server_side_copy: true,
            server_side_move: true,
            // Deleted files can be restored from dropbox.com for 30 days.
            trash: true,
            empty_trash: false,
            native_docs: false,
            hashes: vec!["dropbox".into()],
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions {
            // Dropbox reports changes (longpoll): one minute, like Drive.
            poll_interval_secs: Some(60),
            // Changes are reported, so listings stay valid much longer
            // than rclone's 5 minutes: folders open without a request.
            // An hour bounds how long a missed change can go unseen.
            dir_cache_time_secs: Some(3600),
            ..VfsOptions::default()
        }
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        // Dropbox Paper documents are not exported by rclone.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_params() {
        assert!(Dropbox.config_params(&SetupInput::default()).is_empty());
        let input = SetupInput {
            client_id: Some("app".into()),
            client_secret: Some("secret".into()),
            token: Some("{}".into()),
            ..SetupInput::default()
        };
        let params = Dropbox.config_params(&input);
        assert!(params.contains(&("client_id".into(), "app".into())));
        assert!(params.contains(&("client_secret".into(), "secret".into())));
        assert!(params.contains(&("token".into(), "{}".into())));
    }

    #[test]
    fn info() {
        let info = Dropbox.info();
        assert_eq!(info.oauth_client, Some(CustomClient::Optional));
        assert!(info.capabilities.quota);
    }
}
