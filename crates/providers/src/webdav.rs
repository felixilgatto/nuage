//! WebDAV provider: any WebDAV server (ownCloud, a NAS, Fastmail…). For
//! Nextcloud, the `nextcloud` provider is better (chunked uploads). User
//! documentation: docs/PROVIDERS.md.

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

pub const URL_KEY: &str = "url";
pub const USER_KEY: &str = "user";
pub const PASSWORD_KEY: &str = "pass";

#[derive(Debug, Clone, Copy, Default)]
pub struct WebDav;

impl Provider for WebDav {
    fn id(&self) -> &'static str {
        "webdav"
    }

    fn display_name(&self) -> &str {
        "WebDAV"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "webdav"
    }

    fn auth(&self) -> AuthSpec {
        AuthSpec::Credentials {
            fields: vec![
                crate::field(URL_KEY, "WebDAV Address", false, true),
                crate::field(USER_KEY, "User Name", false, false),
                crate::field(PASSWORD_KEY, "Password", true, false),
            ],
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        let mut params = vec![("vendor".to_owned(), "other".to_owned())];
        for key in [URL_KEY, USER_KEY, PASSWORD_KEY] {
            if let Some(value) = crate::credential(input, key) {
                params.push((key.to_owned(), value.to_owned()));
            }
        }
        params
    }

    fn validate(&self, input: &SetupInput) -> Result<(), String> {
        let url =
            crate::credential(input, URL_KEY).ok_or("Enter the WebDAV address of the server.")?;
        crate::check_web_address(url)?;
        if crate::credential(input, PASSWORD_KEY).is_some()
            && crate::credential(input, USER_KEY).is_none()
        {
            return Err("Enter the user name that goes with the password.".to_owned());
        }
        Ok(())
    }

    fn static_capabilities(&self) -> Capabilities {
        // What every WebDAV server does; operations/fsinfo adds the rest.
        Capabilities {
            quota: false,
            public_link: false,
            server_side_copy: true,
            server_side_move: true,
            trash: false,
            empty_trash: false,
            native_docs: false,
            hashes: Vec::new(),
        }
    }

    fn recommended_vfs_opts(&self) -> VfsOptions {
        VfsOptions {
            // WebDAV has no change notifications: look for changes made
            // elsewhere every minute.
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

    fn input(pairs: &[(&str, &str)]) -> SetupInput {
        SetupInput {
            credentials: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            ..SetupInput::default()
        }
    }

    #[test]
    fn config_params() {
        let params = WebDav.config_params(&input(&[
            (URL_KEY, "https://dav.example.com/files"),
            (USER_KEY, ""),
        ]));
        assert_eq!(
            params,
            vec![
                ("vendor".to_owned(), "other".to_owned()),
                ("url".to_owned(), "https://dav.example.com/files".to_owned()),
            ]
        );
    }

    #[test]
    fn validate() {
        assert!(
            WebDav
                .validate(&input(&[(URL_KEY, "http://nas.local:5005")]))
                .is_ok()
        );
        assert!(WebDav.validate(&input(&[(URL_KEY, "nas.local")])).is_err());
        assert!(
            WebDav
                .validate(&input(&[(URL_KEY, "https://x.org"), (PASSWORD_KEY, "p")]))
                .is_err()
        );
    }
}
