//! Accounts: add, list, rename, reconnect, remove.
//!
//! An account is a record in `config.toml` plus a remote in `rclone.conf`
//! named `nuage-<id>`. The id is random and never changes; users refer to
//! accounts by their name.

use std::io::Read;

use serde_json::{Map, Value};

use nuage_core::{
    Account, AccountAddArgs, AccountId, AuthSpec, CustomClient, Error, Event, Result, SetupInput,
    SyncState,
};

use crate::Inner;
use crate::config::AccountRecord;
use crate::mounts::{remote_fs, validate_mount_point};

fn random_id() -> Result<String> {
    let mut buf = [0u8; 4];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

fn check_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidArgument(
            "An account name cannot be empty.".to_owned(),
        ));
    }
    if trimmed != name {
        return Err(Error::InvalidArgument(
            "An account name cannot start or end with a space.".to_owned(),
        ));
    }
    if name.contains(':') || name.contains('/') || name.chars().any(char::is_control) {
        return Err(Error::InvalidArgument(format!(
            "\"{name}\" is not a valid account name: it cannot contain ':', '/' or control characters."
        )));
    }
    Ok(())
}

/// `base`, or `base 2`, `base 3`… if taken.
fn unique_name(base: &str, taken: &[&str]) -> String {
    let is_taken = |n: &str| taken.iter().any(|t| t.eq_ignore_ascii_case(n));
    if !is_taken(base) {
        return base.to_owned();
    }
    (2..)
        .map(|i| format!("{base} {i}"))
        .find(|n| !is_taken(n))
        .expect("an unused name exists")
}

impl Inner {
    pub(crate) async fn list_accounts(&self) -> Vec<Account> {
        let st = self.state.lock().await;
        st.config
            .accounts
            .iter()
            .map(|r| r.account.clone())
            .collect()
    }

    /// Adds an account following its provider's `AuthSpec`, then mounts it.
    /// Nothing is kept if any step fails.
    pub(crate) async fn add_account(&self, args: AccountAddArgs) -> Result<Account> {
        let provider = self.registry.get(&args.provider).ok_or_else(|| {
            let known: Vec<_> = self.registry.iter().map(|p| p.id()).collect();
            Error::InvalidArgument(format!(
                "Unknown provider \"{}\". Available providers: {}.",
                args.provider,
                known.join(", ")
            ))
        })?;
        let rc = self.rc().await?;
        let mut st = self.state.lock().await;

        let taken: Vec<&str> = st
            .config
            .accounts
            .iter()
            .map(|r| r.account.display_name.as_str())
            .collect();
        let name = match args.name {
            Some(name) => {
                check_name(&name)?;
                if taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
                    return Err(Error::AlreadyExists(format!(
                        "An account named \"{name}\" already exists. Choose another name with --name."
                    )));
                }
                name
            }
            None => unique_name(provider.display_name(), &taken),
        };
        let others: Vec<&Account> = st.config.accounts.iter().map(|r| &r.account).collect();
        let mount_point = validate_mount_point(
            &args
                .mount_point
                .unwrap_or_else(|| self.paths.default_mount_point(&name)),
            &self.paths,
            &others,
        )?;

        let input = SetupInput {
            name: name.clone(),
            token: args.token,
            client_id: args.client_id,
            client_secret: args.client_secret,
            credentials: args.params,
        };
        match provider.auth() {
            AuthSpec::None => {}
            AuthSpec::Credentials { fields } => {
                let missing: Vec<String> = fields
                    .iter()
                    .filter(|f| f.required && !input.credentials.contains_key(&f.key))
                    .map(|f| format!("--param {}=<{}>", f.key, f.label.to_lowercase()))
                    .collect();
                if !missing.is_empty() {
                    return Err(Error::InvalidArgument(format!(
                        "{} needs: {}.",
                        provider.display_name(),
                        missing.join(" ")
                    )));
                }
            }
            AuthSpec::OAuthBrowser { custom_client } => {
                if custom_client == CustomClient::Required
                    && (input.client_id.is_none() || input.client_secret.is_none())
                {
                    return Err(Error::InvalidArgument(format!(
                        "{} needs your own OAuth client: pass --client-id and --client-secret. The README explains how to create one.",
                        provider.display_name()
                    )));
                }
                if input.token.is_none() {
                    return Err(Error::not_implemented(format!(
                        "signing in to {} from Nuage; for now, create a token with `rclone authorize` and pass it with --token-file",
                        provider.display_name()
                    )));
                }
            }
        }
        provider.validate(&input).map_err(Error::InvalidArgument)?;

        let id = AccountId::new(random_id()?);
        let remote_name = format!("nuage-{id}");
        let mut parameters: Map<String, Value> = provider
            .config_params(&input)
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        if input.token.is_some() {
            // The token is final: do not ask rclone to refresh it in a browser.
            parameters.insert("config_refresh_token".into(), "false".into());
        }
        let outcome = rc
            .config_create(&remote_name, provider.rclone_backend(), &parameters)
            .await;
        let failure = match &outcome {
            Ok(o) if !o.error.is_empty() => Some(o.error.clone()),
            Ok(o) if !o.is_complete() => Some(format!(
                "rclone still asks \"{}\"",
                o.pending_question().unwrap_or(&o.state)
            )),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(message) = failure {
            // config/create saves the remote before the questions end.
            let _ = rc.config_delete(&remote_name).await;
            return Err(Error::Rclone(format!(
                "Cannot set up the {} account: {message}",
                provider.display_name()
            )));
        }

        let account = Account {
            id,
            provider_id: provider.id().to_owned(),
            display_name: name,
            remote_name,
            mount_point,
            cache_limit: st.config.settings.default_cache_limit,
        };
        st.config.accounts.push(AccountRecord {
            account: account.clone(),
            paused: false,
            auto_mount: true,
        });
        let i = st.config.accounts.len() - 1;
        let mounted = match self.save(&st) {
            Ok(()) => self.mount_locked(&mut st, i).await,
            Err(e) => Err(e),
        };
        if let Err(e) = mounted {
            st.config.accounts.pop();
            st.runtime.remove(&account.id);
            let _ = self.save(&st);
            let _ = rc.config_delete(&account.remote_name).await;
            let _ = std::fs::remove_dir(&account.mount_point);
            let _ = std::fs::remove_dir(
                crate::MountPlan::layered(&account, &self.paths).rclone_mount_point,
            );
            return Err(e);
        }
        tracing::info!(account = %account.display_name, provider = %account.provider_id, "account added");
        self.emit(Event::AccountAdded {
            account: account.clone(),
        });
        Ok(account)
    }

    pub(crate) async fn rename_account(&self, id: &AccountId, name: &str) -> Result<Account> {
        check_name(name)?;
        let mut st = self.state.lock().await;
        let i = st.resolve(id)?;
        let clash = st
            .config
            .accounts
            .iter()
            .enumerate()
            .any(|(j, r)| j != i && r.account.display_name.eq_ignore_ascii_case(name));
        if clash {
            return Err(Error::AlreadyExists(format!(
                "An account named \"{name}\" already exists."
            )));
        }
        st.config.accounts[i].account.display_name = name.to_owned();
        self.save(&st)?;
        let account = st.record(i).account.clone();
        self.emit(Event::AccountUpdated {
            account: account.clone(),
        });
        Ok(account)
    }

    pub(crate) async fn reconnect_account(&self, id: &AccountId) -> Result<()> {
        let st = self.state.lock().await;
        let i = st.resolve(id)?;
        let account = &st.record(i).account;
        match self.registry.get(&account.provider_id).map(|p| p.auth()) {
            Some(AuthSpec::OAuthBrowser { .. }) => Err(Error::not_implemented(
                "signing in again from Nuage; for now, remove the account and add it with a new token",
            )),
            _ => Err(Error::Unsupported(format!(
                "\"{}\" does not use a sign-in, there is nothing to reconnect.",
                account.display_name
            ))),
        }
    }

    /// Unmounts, then deletes the remote, the cache and the mount folder if
    /// empty. Nothing is deleted in the cloud.
    pub(crate) async fn remove_account(&self, id: &AccountId, force: bool) -> Result<()> {
        let account = {
            let st = self.state.lock().await;
            st.record(st.resolve(id)?).account.clone()
        };
        if !force {
            let report = self.poll_status().await;
            let pending = report
                .accounts
                .iter()
                .find(|a| a.account.id == account.id)
                .map_or(0, |a| match a.state {
                    SyncState::Syncing { uploads, queued } => uploads + queued,
                    SyncState::Paused { queued } => queued,
                    _ => 0,
                });
            if pending > 0 {
                return Err(Error::Busy(format!(
                    "{pending} file(s) of \"{}\" are not uploaded yet. Wait for them, or remove anyway with --force (they are lost).",
                    account.display_name
                )));
            }
        }

        let mut st = self.state.lock().await;
        let i = st.resolve(&account.id)?;
        // rclone names the cache after the backend it wraps (an alias of a
        // local folder is cached under that folder's path): ask it.
        let mut cache_dirs = self.paths.remote_cache_dirs(&account.remote_name).to_vec();
        if st.is_mounted(&account.id)
            && let Ok(stats) = self.rc().await?.vfs_stats(&remote_fs(&account)).await
            && let Some(cache) = stats.disk_cache
        {
            cache_dirs.extend(
                [cache.path, cache.path_meta]
                    .into_iter()
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from)
                    // Never delete anything outside the cache directory.
                    .filter(|p| p.starts_with(&self.paths.cache_dir) && p != &self.paths.cache_dir),
            );
        }
        self.unmount_locked(&mut st, &account).await?;
        let rc = self.rc().await?;
        rc.config_delete(&account.remote_name).await.map_err(|e| {
            Error::Rclone(format!(
                "Cannot delete the rclone remote of \"{}\": {e}",
                account.display_name
            ))
        })?;
        st.config.accounts.remove(i);
        st.runtime.remove(&account.id);
        self.save(&st)?;
        drop(st);

        for dir in cache_dirs {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "cannot delete the cache")
                }
            }
        }
        // Only removed if empty: never delete user files.
        let _ = std::fs::remove_dir(&account.mount_point);
        let _ = std::fs::remove_dir(
            crate::MountPlan::layered(&account, &self.paths).rclone_mount_point,
        );
        tracing::info!(account = %account.display_name, fs = %remote_fs(&account), "account removed");
        self.emit(Event::AccountRemoved {
            account: account.id.clone(),
        });
        self.poll_status().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(unique_name("Google Drive", &[]), "Google Drive");
        assert_eq!(
            unique_name("Google Drive", &["google drive", "Google Drive 2"]),
            "Google Drive 3"
        );
        assert!(check_name("Work").is_ok());
        for bad in ["", " Work", "a:b", "a/b", "tab\there"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }
}
