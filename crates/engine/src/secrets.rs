//! Protection of `rclone.conf`, which holds the OAuth tokens.
//!
//! With a keyring (the Secret portal in the Flatpak, the Secret Service
//! natively), `rclone.conf` is encrypted with a random password stored in
//! it, passed to rclone as `RCLONE_CONFIG_PASS`. Without one (a headless
//! server), the file stays plain, readable by the user only (`0600`), and
//! `nuage status` says so. `NUAGE_KEYRING=off` forces the second mode.

use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use nuage_core::{Error, Result};

/// How long the keyring may take to answer (it may ask to be unlocked).
const KEYRING_TIMEOUT: Duration = Duration::from_secs(10);

const ENCRYPTED_HEADER: &str = "# Encrypted rclone configuration File";

/// How `rclone.conf` is protected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Protection {
    /// Encrypted with this password.
    Encrypted(String),
    /// Plain text, mode 0600.
    Plain,
}

fn attributes(config: &Path) -> HashMap<String, String> {
    HashMap::from([
        ("application".to_owned(), nuage_core::APP_ID.to_owned()),
        ("purpose".to_owned(), "rclone-config".to_owned()),
        ("path".to_owned(), config.display().to_string()),
    ])
}

/// The password from the keyring, created if missing.
async fn keyring_password(config: &Path) -> std::result::Result<String, String> {
    let keyring = oo7::Keyring::new().await.map_err(|e| e.to_string())?;
    keyring.unlock().await.map_err(|e| e.to_string())?;
    let attrs = attributes(config);
    let items = keyring
        .search_items(&attrs)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(item) = items.first() {
        let secret = item.secret().await.map_err(|e| e.to_string())?;
        return String::from_utf8(secret.as_bytes().to_vec())
            .map_err(|_| "the stored password is not text".to_owned());
    }
    let mut bytes = [0u8; 32];
    std::io::Read::read_exact(
        &mut std::fs::File::open("/dev/urandom").map_err(|e| e.to_string())?,
        &mut bytes,
    )
    .map_err(|e| e.to_string())?;
    let password: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    keyring
        .create_item(
            "Nuage: password of the rclone configuration",
            &attrs,
            password.as_str(),
            true,
        )
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!("created the rclone configuration password in the keyring");
    Ok(password)
}

fn is_encrypted(config: &Path) -> bool {
    std::fs::read_to_string(config).is_ok_and(|t| t.starts_with(ENCRYPTED_HEADER))
}

fn restrict(config: &Path) -> Result<()> {
    if config.exists() {
        std::fs::set_permissions(config, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Whether to use the keyring (`NUAGE_KEYRING=off` disables it).
pub(crate) fn keyring_enabled() -> bool {
    std::env::var("NUAGE_KEYRING").map_or(true, |v| v != "off")
}

/// Decides how `rclone.conf` is protected and makes it so: encrypts a plain
/// file once a keyring password is available.
pub(crate) async fn protect(config: &Path, rclone: &Path, use_keyring: bool) -> Result<Protection> {
    let password = if !use_keyring {
        None
    } else {
        match tokio::time::timeout(KEYRING_TIMEOUT, keyring_password(config)).await {
            Ok(Ok(password)) => Some(password),
            Ok(Err(e)) => {
                tracing::info!(error = %e, "no keyring available");
                None
            }
            Err(_) => {
                tracing::warn!("the keyring did not answer in time");
                None
            }
        }
    };

    let Some(password) = password else {
        if is_encrypted(config) {
            return Err(Error::AuthRequired(format!(
                "{} is encrypted, but the keyring holding its password is not available. Unlock your keyring (log in to the desktop), then start nuaged again.",
                config.display()
            )));
        }
        restrict(config)?;
        tracing::info!(path = %config.display(), "rclone.conf is not encrypted (no keyring); it is readable by this user only");
        return Ok(Protection::Plain);
    };

    if !is_encrypted(config) {
        if let Some(dir) = config.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if !config.exists() {
            std::fs::write(config, "")?;
        }
        restrict(config)?;
        let output = tokio::process::Command::new(rclone)
            .args(["config", "encryption", "set"])
            .args(["--password-command", "printenv NUAGE_RCLONE_CONFIG_PASS"])
            .env("RCLONE_CONFIG", config)
            .env("NUAGE_RCLONE_CONFIG_PASS", &password)
            .env_remove("RCLONE_CONFIG_PASS")
            .stdin(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| Error::Rclone(format!("cannot run {}: {e}", rclone.display())))?;
        if !output.status.success() || !is_encrypted(config) {
            return Err(Error::Rclone(format!(
                "cannot encrypt {}: {}",
                config.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        tracing::info!(path = %config.display(), "rclone.conf is now encrypted");
    }
    restrict(config)?;
    Ok(Protection::Encrypted(password))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plain_without_keyring() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("rclone.conf");
        std::fs::write(&config, "[a]\ntype = alias\n").unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o644)).unwrap();
        let rclone = Path::new("/nonexistent/rclone");
        assert_eq!(
            protect(&config, rclone, false).await.unwrap(),
            Protection::Plain
        );
        let mode = std::fs::metadata(&config).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        std::fs::write(
            &config,
            format!("{ENCRYPTED_HEADER}\n\nRCLONE_ENCRYPT_V0:\nxx"),
        )
        .unwrap();
        let err = protect(&config, rclone, false).await.unwrap_err();
        assert!(matches!(err, Error::AuthRequired(_)), "{err:?}");
    }
}
