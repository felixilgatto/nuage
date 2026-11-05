//! Nextcloud provider: the files of a Nextcloud account, over WebDAV, with
//! Nextcloud's chunked uploads. User documentation: docs/PROVIDERS.md.

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

/// Key of [`SetupInput::credentials`]: the server address, or the full
/// WebDAV address shown in Nextcloud (Files → Settings).
pub const SERVER_KEY: &str = "server";
pub const USER_KEY: &str = "user";
pub const PASSWORD_KEY: &str = "pass";

#[derive(Debug, Clone, Copy, Default)]
pub struct Nextcloud;

impl Provider for Nextcloud {
    fn id(&self) -> &'static str {
        "nextcloud"
    }

    fn display_name(&self) -> &str {
        "Nextcloud"
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
                crate::field(SERVER_KEY, "Server Address", false, true),
                crate::field(USER_KEY, "User Name", false, true),
                // With two-factor authentication, only an app password works.
                crate::field(PASSWORD_KEY, "App Password", true, true),
            ],
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        let get = |key| crate::credential(input, key).unwrap_or_default();
        let user = get(USER_KEY);
        vec![
            ("url".to_owned(), dav_url(get(SERVER_KEY), user)),
            ("vendor".to_owned(), "nextcloud".to_owned()),
            ("user".to_owned(), user.to_owned()),
            ("pass".to_owned(), get(PASSWORD_KEY).to_owned()),
        ]
    }

    fn validate(&self, input: &SetupInput) -> Result<(), String> {
        let server = crate::credential(input, SERVER_KEY)
            .ok_or("Enter the address of your Nextcloud server.")?;
        crate::check_web_address(server)?;
        crate::credential(input, USER_KEY).ok_or("Enter your Nextcloud user name.")?;
        crate::credential(input, PASSWORD_KEY).ok_or(
            "Enter an app password: in Nextcloud, Personal settings → Security → Create new app password.",
        )?;
        Ok(())
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: false,
            server_side_copy: true,
            server_side_move: true,
            // Deleted files go to Nextcloud's "Deleted files".
            trash: true,
            empty_trash: false,
            native_docs: false,
            hashes: vec!["sha1".into(), "md5".into()],
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

/// The WebDAV address of the user's files: the address as given when it
/// already is one, otherwise `<server>/remote.php/dav/files/<user>/`.
fn dav_url(server: &str, user: &str) -> String {
    let server = server.trim_end_matches('/');
    if server.contains("/remote.php/") {
        return format!("{server}/");
    }
    format!("{server}/remote.php/dav/files/{}/", encode_segment(user))
}

/// Percent-encodes a path segment.
fn encode_segment(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'@' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(server: &str) -> SetupInput {
        SetupInput {
            credentials: [
                (SERVER_KEY.to_owned(), server.to_owned()),
                (USER_KEY.to_owned(), "Ana María".to_owned()),
                (PASSWORD_KEY.to_owned(), "app-password".to_owned()),
            ]
            .into(),
            ..SetupInput::default()
        }
    }

    #[test]
    fn url_from_server() {
        let params = Nextcloud.config_params(&input("https://cloud.example.com/"));
        assert!(params.contains(&(
            "url".into(),
            "https://cloud.example.com/remote.php/dav/files/Ana%20Mar%C3%ADa/".into()
        )));
        assert!(params.contains(&("vendor".into(), "nextcloud".into())));
        assert!(params.contains(&("pass".into(), "app-password".into())));
    }

    #[test]
    fn url_given_whole() {
        let url = "https://example.com/nc/remote.php/dav/files/ana";
        let params = Nextcloud.config_params(&input(url));
        assert!(params.contains(&("url".into(), format!("{url}/"))));
    }

    #[test]
    fn validate() {
        assert!(
            Nextcloud
                .validate(&input("https://cloud.example.com"))
                .is_ok()
        );
        assert!(Nextcloud.validate(&input("cloud.example.com")).is_err());
        assert!(Nextcloud.validate(&SetupInput::default()).is_err());
        let mut no_password = input("https://cloud.example.com");
        no_password
            .credentials
            .insert(PASSWORD_KEY.into(), " ".into());
        assert!(Nextcloud.validate(&no_password).is_err());
    }
}
