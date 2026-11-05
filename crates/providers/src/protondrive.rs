//! Proton Drive provider, through rclone's `protondrive` backend (beta in
//! rclone). Experimental: not in the default features. User
//! documentation: docs/PROVIDERS.md.
//!
//! rclone signs in with the password once and then keeps the session in
//! the remote's configuration; a session Proton ends cannot be reconnected
//! (credentials accounts have no sign-in to redo): the account is removed
//! and added again.

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

pub const USER_KEY: &str = "username";
pub const PASSWORD_KEY: &str = "password";
/// The six-digit code of the authenticator app, if two-factor
/// authentication is on. Only used for the first sign-in.
pub const CODE_KEY: &str = "2fa";

#[derive(Debug, Clone, Copy, Default)]
pub struct ProtonDrive;

impl Provider for ProtonDrive {
    fn id(&self) -> &'static str {
        "protondrive"
    }

    fn display_name(&self) -> &str {
        "Proton Drive"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "protondrive"
    }

    fn auth(&self) -> AuthSpec {
        AuthSpec::Credentials {
            fields: vec![
                crate::field(USER_KEY, "Proton Address", false, true),
                crate::field(PASSWORD_KEY, "Password", true, true),
                crate::field(CODE_KEY, "Two-Factor Code (If Enabled)", false, false),
            ],
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        let mut params = Vec::new();
        for key in [USER_KEY, PASSWORD_KEY, CODE_KEY] {
            if let Some(value) = crate::credential(input, key) {
                params.push((key.to_owned(), value.to_owned()));
            }
        }
        params
    }

    fn validate(&self, input: &SetupInput) -> Result<(), String> {
        crate::credential(input, USER_KEY).ok_or("Enter your Proton address.")?;
        crate::credential(input, PASSWORD_KEY).ok_or("Enter your Proton password.")?;
        if let Some(code) = crate::credential(input, CODE_KEY)
            && !(code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(
                "The two-factor code is the six digits shown by your authenticator app.".to_owned(),
            );
        }
        Ok(())
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: true,
            public_link: false,
            server_side_copy: false,
            server_side_move: true,
            // rclone moves deleted files to the Proton Drive trash.
            trash: true,
            empty_trash: true,
            native_docs: false,
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
        // Proton Docs are not exported by rclone.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(code: &str) -> SetupInput {
        SetupInput {
            credentials: [
                (USER_KEY.to_owned(), "ana@proton.me".to_owned()),
                (PASSWORD_KEY.to_owned(), "secret".to_owned()),
                (CODE_KEY.to_owned(), code.to_owned()),
            ]
            .into(),
            ..SetupInput::default()
        }
    }

    #[test]
    fn config_params() {
        let params = ProtonDrive.config_params(&input(""));
        assert_eq!(params.len(), 2, "no empty 2FA code");
        assert!(params.contains(&("password".into(), "secret".into())));
        assert!(
            ProtonDrive
                .config_params(&input("123456"))
                .contains(&("2fa".into(), "123456".into()))
        );
    }

    #[test]
    fn validate() {
        assert!(ProtonDrive.validate(&input("")).is_ok());
        assert!(ProtonDrive.validate(&input("123456")).is_ok());
        assert!(ProtonDrive.validate(&input("12345")).is_err());
        assert!(ProtonDrive.validate(&SetupInput::default()).is_err());
    }
}
