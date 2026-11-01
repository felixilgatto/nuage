//! Google Drive provider ("My Drive" only in v1). User documentation:
//! docs/PROVIDERS.md and docs/GOOGLE_OAUTH.md.

use nuage_core::{
    AuthSpec, Capabilities, CustomClient, GuideStep, NativeDocPolicy, SetupInput, VfsOptions,
};

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
        // Google Docs, Sheets, Slides… as web shortcuts that open in the
        // browser (rclone's export format `link.html`).
        if let Some(NativeDocPolicy::WebShortcut { export_formats, .. }) = self.native_docs() {
            params.push(("export_formats".to_owned(), export_formats));
        }
        // v1 mounts "My Drive" only: do not ask about shared drives.
        params.push(("config_change_team_drive".to_owned(), "false".to_owned()));
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

    fn client_guide(&self) -> Vec<GuideStep> {
        let step = |title: &str, url: &str| GuideStep {
            title: title.to_owned(),
            url: url.to_owned(),
        };
        // Same steps as docs/GOOGLE_OAUTH.md.
        vec![
            step(
                "Create a project in the Google Cloud console",
                "https://console.cloud.google.com/projectcreate",
            ),
            step(
                "Enable the Google Drive API",
                "https://console.cloud.google.com/apis/library/drive.googleapis.com",
            ),
            step(
                "Describe the app: a name, your e-mail address, audience External",
                "https://console.cloud.google.com/auth/branding",
            ),
            step(
                "Publish the app, or Google asks you to sign in every week",
                "https://console.cloud.google.com/auth/audience",
            ),
            step(
                "Create a client of type Desktop app, then copy its ID and secret below",
                "https://console.cloud.google.com/auth/clients",
            ),
        ]
    }

    fn client_id_suffix(&self) -> Option<&'static str> {
        Some(".apps.googleusercontent.com")
    }

    fn native_docs(&self) -> Option<NativeDocPolicy> {
        // `link.html` opens in the default browser from Nautilus; `desktop`
        // opens in a text editor. The link files are read-only.
        Some(NativeDocPolicy::WebShortcut {
            export_formats: "link.html".to_owned(),
            suffix: ".link.html".to_owned(),
            web_url: Some("https://drive.google.com/open?id={id}".to_owned()),
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
        assert!(params.contains(&("export_formats".to_owned(), "link.html".to_owned())));
        assert!(params.contains(&("config_change_team_drive".to_owned(), "false".to_owned())));
        assert!(!params.iter().any(|(k, _)| k == "token"));
        assert_eq!(GoogleDrive.rclone_backend(), "drive");
    }
}
