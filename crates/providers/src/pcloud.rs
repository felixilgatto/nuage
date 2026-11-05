//! pCloud provider (the whole pCloud of the signed-in account, in the US or
//! the EU region). User documentation: docs/PROVIDERS.md.

use nuage_core::{AuthSpec, Capabilities, CustomClient, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

#[derive(Debug, Clone, Copy, Default)]
pub struct PCloud;

impl Provider for PCloud {
    fn id(&self) -> &'static str {
        "pcloud"
    }

    fn display_name(&self) -> &str {
        "pCloud"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "pcloud"
    }

    fn auth(&self) -> AuthSpec {
        // rclone's own pCloud client works; the sign-in tells rclone the
        // account's region (US or EU) by itself.
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
            // Deleted files go to the pCloud trash.
            trash: true,
            empty_trash: true,
            native_docs: false,
            // md5 only exists in the US region.
            hashes: vec!["sha1".into()],
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions {
            // No change notifications: look for changes made elsewhere
            // every minute.
            dir_cache_time_secs: Some(60),
            ..VfsOptions::default()
        }
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_params() {
        assert!(PCloud.config_params(&SetupInput::default()).is_empty());
        let input = SetupInput {
            token: Some("{}".into()),
            ..SetupInput::default()
        };
        assert_eq!(
            PCloud.config_params(&input),
            vec![("token".to_owned(), "{}".to_owned())]
        );
    }

    #[test]
    fn info() {
        let info = PCloud.info();
        assert_eq!(info.oauth_client, Some(CustomClient::Optional));
        assert!(info.client_guide.is_empty());
    }
}
