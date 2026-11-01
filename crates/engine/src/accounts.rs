//! Accounts: add, list, rename, reconnect, remove.
//!
//! An account is a record in `config.toml` plus a remote in `rclone.conf`
//! named `nuage-<id>`. The id is random and never changes; users refer to
//! accounts by their name.

use std::io::Read;

use serde_json::{Map, Value};

use nuage_core::{
    Account, AccountAddArgs, AccountId, AuthSpec, CustomClient, Error, Event, Reply, Result,
    SetupInput, SyncState,
};
use nuage_rclone::RcClient;

use crate::config::AccountRecord;
use crate::mounts::{remote_fs, validate_mount_point};
use crate::signin::{BROWSER_QUESTION, SignInKind};
use crate::{Inner, State};

fn random_id() -> Result<String> {
    let mut buf = [0u8; 4];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Number of files under `dir`.
fn walk_files(dir: &std::path::Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk_files(&e.path())
            } else {
                1
            }
        })
        .sum()
}

/// The `config_*` parameters, which answer rclone's configuration questions.
fn config_answers(parameters: &Map<String, Value>) -> Vec<(String, String)> {
    parameters
        .iter()
        // Never answer the sign-in questions themselves again.
        .filter(|(k, _)| {
            k.starts_with("config_") && *k != "config_refresh_token" && *k != BROWSER_QUESTION
        })
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
        .collect()
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
    ///
    /// Replies [`Reply::Account`], or [`Reply::SignIn`] when the user must
    /// sign in in a browser first (see [`crate::signin`]).
    pub(crate) async fn add_account(&self, args: AccountAddArgs) -> Result<Reply> {
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

        let mut input = SetupInput {
            name: name.clone(),
            token: args.token,
            client_id: args.client_id,
            client_secret: args.client_secret,
            credentials: args.params,
        };
        let mut sign_in = false;
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
                // The client of an existing account of the same provider is
                // reused: a second account does not ask for it again.
                if input.client_id.is_none() && custom_client != CustomClient::Unsupported {
                    let existing = st
                        .config
                        .accounts
                        .iter()
                        .find(|r| r.account.provider_id == provider.id())
                        .map(|r| r.account.remote_name.clone());
                    if let Some(remote) = existing
                        && let Ok(stored) = rc.config_get(&remote).await
                    {
                        let get = |k: &str| {
                            stored
                                .get(k)
                                .and_then(Value::as_str)
                                .filter(|v| !v.is_empty())
                                .map(str::to_owned)
                        };
                        input.client_id = get("client_id");
                        input.client_secret = get("client_secret");
                    }
                }
                if custom_client == CustomClient::Required
                    && (input.client_id.is_none() || input.client_secret.is_none())
                {
                    return Err(Error::InvalidArgument(format!(
                        "{} needs your own OAuth client: pass --client-id and --client-secret. The README explains how to create one.",
                        provider.display_name()
                    )));
                }
                if input.token.is_none() {
                    self.check_no_sign_in().await?;
                    sign_in = true;
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
        if sign_in {
            // Nuage shows the page; rclone must not try to open a browser.
            parameters.insert("config_auth_no_browser".into(), "true".into());
        }
        let outcome = rc
            .config_create(&remote_name, provider.rclone_backend(), &parameters)
            .await;
        let failure = match &outcome {
            Ok(o) if !o.error.is_empty() => Some(o.error.clone()),
            Ok(o) if sign_in && o.pending_question() == Some(BROWSER_QUESTION) => None,
            Ok(o) if !o.is_complete() => Some(format!(
                "rclone still asks \"{}\"",
                o.pending_question().unwrap_or(&o.state)
            )),
            Ok(_) if sign_in => Some("rclone did not ask to sign in".to_owned()),
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
            remote_name: remote_name.clone(),
            mount_point,
            cache_limit: st.config.settings.default_cache_limit,
        };
        if sign_in {
            drop(st);
            let state = outcome.map(|o| o.state).unwrap_or_default();
            let answers = config_answers(&parameters);
            let sign_in = self
                .start_sign_in(&rc, &remote_name, &state, SignInKind::Add(account), answers)
                .await?;
            return Ok(Reply::SignIn(sign_in));
        }
        self.finish_add(&mut st, &rc, account)
            .await
            .map(Reply::Account)
    }

    /// Saves and mounts a new account whose remote exists; undoes everything
    /// if that fails.
    pub(crate) async fn finish_add(
        &self,
        st: &mut State,
        rc: &RcClient,
        account: Account,
    ) -> Result<Account> {
        st.config.accounts.push(AccountRecord {
            account: account.clone(),
            paused: false,
            auto_mount: true,
            default_exclusions: true,
            exclusions: Vec::new(),
        });
        let i = st.config.accounts.len() - 1;
        let mounted = match self.save(st) {
            Ok(()) => self.mount_locked(st, i).await,
            Err(e) => Err(e),
        };
        if let Err(e) = mounted {
            st.config.accounts.pop();
            st.runtime.remove(&account.id);
            let _ = self.save(st);
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

    /// Signs in again: with `token`, at once; otherwise in a browser
    /// ([`Reply::SignIn`]). The account is remounted to use the new token.
    pub(crate) async fn reconnect_account(
        &self,
        id: &AccountId,
        token: Option<String>,
    ) -> Result<Reply> {
        let rc = self.rc().await?;
        let account = {
            let st = self.state.lock().await;
            st.record(st.resolve(id)?).account.clone()
        };
        let provider = self.registry.get(&account.provider_id);
        let Some(provider) = provider.filter(|p| matches!(p.auth(), AuthSpec::OAuthBrowser { .. }))
        else {
            return Err(Error::Unsupported(format!(
                "\"{}\" does not use a sign-in, there is nothing to reconnect.",
                account.display_name
            )));
        };
        let stored = rc.config_get(&account.remote_name).await?;
        let get = |k: &str| {
            stored
                .get(k)
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let input = SetupInput {
            name: account.display_name.clone(),
            token: token.clone(),
            client_id: get("client_id"),
            client_secret: get("client_secret"),
            credentials: Default::default(),
        };
        let mut parameters: Map<String, Value> = provider
            .config_params(&input)
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();
        if token.is_some() {
            parameters.insert("config_refresh_token".into(), "false".into());
            let outcome = rc.config_update(&account.remote_name, &parameters).await?;
            if !outcome.error.is_empty() || !outcome.is_complete() {
                return Err(Error::Rclone(format!(
                    "Cannot use the new token: {}",
                    if outcome.error.is_empty() {
                        outcome
                            .pending_question()
                            .unwrap_or(&outcome.state)
                            .to_owned()
                    } else {
                        outcome.error
                    }
                )));
            }
            return self.finish_reconnect(&account.id).await.map(Reply::Account);
        }

        self.check_no_sign_in().await?;
        parameters.insert("config_refresh_token".into(), "true".into());
        parameters.insert("config_auth_no_browser".into(), "true".into());
        let outcome = rc.config_update(&account.remote_name, &parameters).await?;
        if outcome.pending_question() != Some(BROWSER_QUESTION) {
            return Err(Error::Rclone(format!(
                "rclone did not ask to sign in again ({}).",
                if outcome.error.is_empty() {
                    outcome.state
                } else {
                    outcome.error
                }
            )));
        }
        let sign_in = self
            .start_sign_in(
                &rc,
                &account.remote_name,
                &outcome.state,
                SignInKind::Reconnect(account.id.clone()),
                config_answers(&parameters),
            )
            .await?;
        Ok(Reply::SignIn(sign_in))
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

        // Excluded files created locally exist nowhere else.
        let local_only = self.paths.local_only(&account.id);
        let local_files = walk_files(&local_only);
        if local_files > 0 && !force {
            return Err(Error::Busy(format!(
                "{local_files} excluded file(s) of \"{}\" exist only on this computer ({}). Move them elsewhere, or remove anyway with --force (they are deleted).",
                account.display_name,
                local_only.display()
            )));
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
        let _ = std::fs::remove_dir_all(&local_only);
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
