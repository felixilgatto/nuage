//! S3 provider: Amazon S3 and S3-compatible bucket storage (Cloudflare R2,
//! Backblaze B2, Wasabi, MinIO, Scaleway…). User documentation:
//! docs/PROVIDERS.md.
//!
//! The account shows one bucket (or a folder of it) when the user names
//! one, which works with keys limited to that bucket; otherwise every
//! bucket is a folder, and the key must be allowed to list buckets.

use nuage_core::{AuthSpec, Capabilities, NativeDocPolicy, SetupInput, VfsOptions};

use crate::Provider;

/// The service's address; empty for Amazon S3.
pub const ENDPOINT_KEY: &str = "endpoint";
pub const REGION_KEY: &str = "region";
pub const ACCESS_KEY: &str = "access_key_id";
pub const SECRET_KEY: &str = "secret_access_key";
/// The bucket the account shows, or a folder of it (`bucket/folder`);
/// empty for every bucket. Not an rclone parameter: the account root.
pub const BUCKET_KEY: &str = "bucket";

#[derive(Debug, Clone, Copy, Default)]
pub struct S3;

impl Provider for S3 {
    fn id(&self) -> &'static str {
        "s3"
    }

    fn display_name(&self) -> &str {
        "S3 Bucket Storage"
    }

    fn icon_name(&self) -> &str {
        "folder-remote-symbolic"
    }

    fn rclone_backend(&self) -> &'static str {
        "s3"
    }

    fn auth(&self) -> AuthSpec {
        AuthSpec::Credentials {
            fields: vec![
                crate::field(ACCESS_KEY, "Access Key ID", false, true),
                crate::field(SECRET_KEY, "Secret Access Key", true, true),
                crate::field(BUCKET_KEY, "Bucket (Empty for All)", false, false),
                crate::field(ENDPOINT_KEY, "Endpoint (Empty for Amazon S3)", false, false),
                crate::field(REGION_KEY, "Region (If Required)", false, false),
            ],
        }
    }

    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)> {
        let get = |key| crate::credential(input, key);
        let mut params = vec![
            // Only the keys entered, never the environment or the machine's
            // role.
            ("env_auth".to_owned(), "false".to_owned()),
            (
                "access_key_id".to_owned(),
                get(ACCESS_KEY).unwrap_or_default().to_owned(),
            ),
            (
                "secret_access_key".to_owned(),
                get(SECRET_KEY).unwrap_or_default().to_owned(),
            ),
        ];
        match get(ENDPOINT_KEY) {
            None => {
                params.push(("provider".to_owned(), "AWS".to_owned()));
                params.push((
                    "region".to_owned(),
                    get(REGION_KEY).unwrap_or("us-east-1").to_owned(),
                ));
            }
            Some(endpoint) => {
                params.push(("provider".to_owned(), "Other".to_owned()));
                params.push(("endpoint".to_owned(), endpoint.to_owned()));
                if let Some(region) = get(REGION_KEY) {
                    params.push(("region".to_owned(), region.to_owned()));
                }
            }
        }
        params
    }

    fn root(&self, input: &SetupInput) -> String {
        crate::credential(input, BUCKET_KEY)
            .map(|b| b.strip_prefix("s3://").unwrap_or(b).trim_matches('/'))
            .unwrap_or_default()
            .to_owned()
    }

    fn validate(&self, input: &SetupInput) -> Result<(), String> {
        crate::credential(input, ACCESS_KEY)
            .ok_or("Enter the access key ID given by your storage service.")?;
        crate::credential(input, SECRET_KEY)
            .ok_or("Enter the secret access key that goes with the access key ID.")?;
        if let Some(endpoint) = crate::credential(input, ENDPOINT_KEY)
            && endpoint.contains(char::is_whitespace)
        {
            return Err(format!(
                "\"{endpoint}\" is not an endpoint: copy the address your storage service gives, e.g. https://s3.eu-central-003.backblazeb2.com."
            ));
        }
        let root = self.root(input);
        let bucket = root.split('/').next().unwrap_or_default();
        if !bucket.is_empty()
            && !bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.".contains(&b))
        {
            return Err(format!(
                "\"{bucket}\" is not a bucket name: it has only lowercase letters, digits, dots and hyphens."
            ));
        }
        Ok(())
    }

    fn static_capabilities(&self) -> Capabilities {
        Capabilities {
            quota: false,
            // Signed links that expire; operations/fsinfo reports them.
            public_link: false,
            server_side_copy: true,
            // A rename is a copy and a delete.
            server_side_move: false,
            trash: false,
            empty_trash: false,
            native_docs: false,
            hashes: vec!["md5".into()],
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

    fn input(pairs: &[(&str, &str)]) -> SetupInput {
        let mut credentials: std::collections::BTreeMap<String, String> = [
            (ACCESS_KEY.to_owned(), "AK".to_owned()),
            (SECRET_KEY.to_owned(), "SK".to_owned()),
        ]
        .into();
        for (k, v) in pairs {
            credentials.insert((*k).to_owned(), (*v).to_owned());
        }
        SetupInput {
            credentials,
            ..SetupInput::default()
        }
    }

    #[test]
    fn amazon() {
        let params = S3.config_params(&input(&[]));
        assert!(params.contains(&("provider".into(), "AWS".into())));
        assert!(params.contains(&("region".into(), "us-east-1".into())));
        assert!(params.contains(&("env_auth".into(), "false".into())));
        assert!(!params.iter().any(|(k, _)| k == "endpoint"));
    }

    #[test]
    fn compatible() {
        let params = S3.config_params(&input(&[(ENDPOINT_KEY, "https://minio.local:9000")]));
        assert!(params.contains(&("provider".into(), "Other".into())));
        assert!(params.contains(&("endpoint".into(), "https://minio.local:9000".into())));
        assert!(!params.iter().any(|(k, _)| k == "region"));
    }

    #[test]
    fn bucket_root() {
        assert_eq!(S3.root(&input(&[])), "");
        assert_eq!(S3.root(&input(&[(BUCKET_KEY, " photos ")])), "photos");
        assert_eq!(
            S3.root(&input(&[(BUCKET_KEY, "s3://photos/2026/")])),
            "photos/2026"
        );
        let params = S3.config_params(&input(&[(BUCKET_KEY, "photos")]));
        assert!(
            !params.iter().any(|(k, _)| k == BUCKET_KEY),
            "not for rclone"
        );
        assert!(S3.validate(&input(&[(BUCKET_KEY, "my photos")])).is_err());
        assert!(
            S3.validate(&input(&[(BUCKET_KEY, "my-photos.2026/Raw Files")]))
                .is_ok()
        );
    }

    #[test]
    fn validate() {
        assert!(S3.validate(&input(&[])).is_ok());
        assert!(S3.validate(&input(&[(SECRET_KEY, "")])).is_err());
        assert!(
            S3.validate(&input(&[(ENDPOINT_KEY, "my endpoint")]))
                .is_err()
        );
    }
}
