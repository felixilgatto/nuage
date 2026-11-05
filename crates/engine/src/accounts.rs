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

/// How long the first connection of a new account may take.
const CONNECTION_CHECK: std::time::Duration = std::time::Duration::from_secs(30);

fn random_id() -> Result<String> {
    let mut buf = [0u8; 4];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// What the user reads when a new account's form values do not work.
fn connection_error(
    provider: &str,
    root: &str,
    filled: &[String],
    empty: &[String],
    e: &nuage_rclone::RcError,
) -> Error {
    let message = match e {
        nuage_rclone::RcError::Api { message, .. } => message.clone(),
        other => other.to_string(),
    };
    let m = message.to_ascii_lowercase();
    let refused = [
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "access denied",
        "invalidaccesskeyid",
        "signaturedoesnotmatch",
        "incorrect login",
        "invalid password",
        "password is incorrect",
    ]
    .iter()
    .any(|s| m.contains(s));
    if refused {
        let what = match filled {
            [] => "what you entered".to_owned(),
            labels => format!("the {}", list(labels)),
        };
        let or = match empty {
            [] => String::new(),
            labels => format!(", or fill in the {}", list(labels)),
        };
        return Error::InvalidArgument(format!(
            "{provider} refused the connection: check {what}{or}."
        ));
    }
    if crate::files::is_not_found(e) {
        return Error::InvalidArgument(if root.is_empty() {
            format!("{provider} has nothing at this address: check it.")
        } else {
            format!("\"{root}\" does not exist on {provider}: check its name.")
        });
    }
    // A server that does not answer: rclone retries until the timeout.
    if matches!(e, nuage_rclone::RcError::Http(_)) {
        return Error::Rclone(format!(
            "Cannot reach {provider}: it does not answer. Check the address and your connection."
        ));
    }
    if crate::files::is_offline(e) {
        return Error::Rclone(format!(
            "Cannot reach {provider}: check the address and your connection ({message})."
        ));
    }
    Error::Rclone(format!("Cannot connect to {provider}: {message}"))
}

/// `a`, `a and b`, `a, b and c`.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// A form label as it reads in a sentence: "User Name (If Any)" → "user
/// name". Acronyms ("ID", "WebDAV") keep their case.
fn label_in_sentence(label: &str) -> String {
    let label = label.split(" (").next().unwrap_or(label);
    label
        .split(' ')
        .map(|w| {
            if w.chars().skip(1).any(char::is_uppercase) {
                w.to_owned()
            } else {
                w.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        let requested = args
            .mount_point
            .clone()
            .unwrap_or_else(|| self.paths.default_mount_point(&name));
        // Without mounting now (desktop setup), the folder is chosen and
        // checked at the last screen.
        let mount_point = if args.no_mount {
            crate::mounts::normalize(&requested)
        } else {
            validate_mount_point(&requested, &self.paths, &others)?
        };

        let mut input = SetupInput {
            name: name.clone(),
            token: args.token,
            client_id: args.client_id,
            client_secret: args.client_secret,
            credentials: args.params,
        };
        // A drive of an existing account (a shared drive): its sign-in.
        let mut sub_drive: Option<(String, String)> = None;
        if let Some(source) = &args.from_account {
            let spec = provider.sub_drives().ok_or_else(|| {
                Error::Unsupported(format!(
                    "{} accounts have no other drives.",
                    provider.display_name()
                ))
            })?;
            let drive = args.sub_drive.clone().ok_or_else(|| {
                Error::InvalidArgument(format!(
                    "Name the drive to add: one of `nuage account drives \"{}\"`.",
                    source.as_str()
                ))
            })?;
            let record = st.record(st.resolve(source)?);
            if record.account.provider_id != provider.id() {
                return Err(Error::InvalidArgument(format!(
                    "\"{}\" is not a {} account.",
                    record.account.display_name,
                    provider.display_name()
                )));
            }
            let stored = rc.config_get(&record.account.remote_name).await?;
            let get = |k: &str| {
                stored
                    .get(k)
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
                    .map(str::to_owned)
            };
            input.token = get("token");
            input.client_id = get("client_id");
            input.client_secret = get("client_secret");
            if input.token.is_none() {
                return Err(Error::AuthRequired(format!(
                    "\"{}\" must be signed in again first.",
                    record.account.display_name
                )));
            }
            sub_drive = Some((spec.param, drive));
        }
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
                        "{} needs your own OAuth client: pass --client-id and --client-secret. How to create one: https://github.com/felixilgatto/nuage/blob/main/docs/GOOGLE_OAUTH.md",
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
        let root = provider.root(&input);

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
        if let Some((param, drive)) = &sub_drive {
            parameters.insert(param.clone(), drive.clone().into());
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
            // A token was given: rclone asks what it asks after a sign-in
            // (e.g. which OneDrive drive).
            Ok(o) if !sign_in && !o.is_complete() => self
                .answer_questions(
                    &rc,
                    &remote_name,
                    o.clone(),
                    &config_answers(&parameters),
                    provider.id(),
                )
                .await
                .err()
                .map(|e| e.to_string()),
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
        // rclone saves a form's values without trying them: try them now,
        // or a wrong password would give an account that shows nothing.
        if let AuthSpec::Credentials { fields } = provider.auth()
            && let Err(e) = rc
                .list_within(&format!("{remote_name}:{root}"), "", CONNECTION_CHECK)
                .await
        {
            let _ = rc.config_delete(&remote_name).await;
            // One of the fields filled in is wrong, or an optional one left
            // empty is needed (e.g. the bucket of a key limited to it).
            let (filled, empty): (Vec<_>, Vec<_>) =
                fields.iter().filter(|f| !f.folder).partition(|f| {
                    input
                        .credentials
                        .get(&f.key)
                        .is_some_and(|v| !v.trim().is_empty())
                });
            let labels = |fields: Vec<&nuage_core::CredentialField>| -> Vec<String> {
                fields.iter().map(|f| label_in_sentence(&f.label)).collect()
            };
            return Err(connection_error(
                provider.display_name(),
                &root,
                &labels(filled),
                &labels(empty),
                &e,
            ));
        }

        let account = Account {
            id,
            provider_id: provider.id().to_owned(),
            display_name: name,
            remote_name: remote_name.clone(),
            root,
            mount_point,
            cache_limit: st.config.settings.default_cache_limit,
        };
        let mount = !args.no_mount;
        if sign_in {
            drop(st);
            let state = outcome.map(|o| o.state).unwrap_or_default();
            let answers = config_answers(&parameters);
            let sign_in = self
                .start_sign_in(
                    &rc,
                    &remote_name,
                    &state,
                    SignInKind::Add { account, mount },
                    answers,
                    provider.id(),
                )
                .await?;
            return Ok(Reply::SignIn(sign_in));
        }
        self.finish_add(&mut st, &rc, account, mount)
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
        mount: bool,
    ) -> Result<Account> {
        st.config.accounts.push(AccountRecord {
            account: account.clone(),
            paused: false,
            auto_mount: mount,
            default_exclusions: true,
            exclusions: Vec::new(),
            cache_max_age_secs: None,
        });
        let i = st.config.accounts.len() - 1;
        let mounted = match self.save(st) {
            Ok(()) if mount => self.mount_locked(st, i).await,
            other => other,
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

    /// Checks a folder for an account (`for_account`: the account that
    /// would move there, which may keep its own folder).
    pub(crate) async fn validate_mount_point(
        &self,
        for_account: Option<&AccountId>,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf> {
        let st = self.state.lock().await;
        let own = for_account.map(|id| st.resolve(id)).transpose()?;
        // An account's own folder, where it is mounted, is fine.
        if let Some(i) = own
            && st.record(i).account.mount_point == crate::mounts::normalize(path)
            && st.is_mounted(&st.record(i).account.id)
        {
            return Ok(st.record(i).account.mount_point.clone());
        }
        let others: Vec<&Account> = st
            .config
            .accounts
            .iter()
            .enumerate()
            .filter(|(j, _)| Some(*j) != own)
            .map(|(_, r)| &r.account)
            .collect();
        validate_mount_point(path, &self.paths, &others)
    }

    /// Moves an account to another folder.
    pub(crate) async fn set_mount_point(
        &self,
        id: &AccountId,
        path: &std::path::Path,
    ) -> Result<Account> {
        let path = self.validate_mount_point(Some(id), path).await?;
        let mut st = self.state.lock().await;
        let i = st.resolve(id)?;
        let account = st.record(i).account.clone();
        if account.mount_point == path {
            return Ok(account);
        }
        self.unmount_locked(&mut st, &account).await?;
        // Only removed if empty: never delete user files.
        let _ = std::fs::remove_dir(&account.mount_point);
        st.config.accounts[i].account.mount_point = path;
        self.save(&st)?;
        if st.record(i).auto_mount {
            self.mount_locked(&mut st, i).await?;
        }
        let account = st.record(i).account.clone();
        tracing::info!(account = %account.display_name, at = %account.mount_point.display(), "moved");
        self.emit(Event::AccountUpdated {
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
                &account.provider_id,
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

        // Pinned files changed offline exist nowhere else either.
        let (_, pins_pending, _) = self.pins.summary(&account.id);
        if pins_pending > 0 && !force {
            return Err(Error::Busy(format!(
                "{pins_pending} change(s) to items of \"{}\" kept offline are not uploaded yet. Wait for the connection, or remove anyway with --force (they are lost).",
                account.display_name
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

        nuage_rclone::cache_deleted(&account.remote_name);
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
        self.pins_account_removed(&account.id);
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

impl Inner {
    /// The other drives an account gives access to (shared drives).
    pub(crate) async fn list_sub_drives(
        &self,
        id: &AccountId,
    ) -> Result<Vec<nuage_core::SubDrive>> {
        let account = {
            let st = self.state.lock().await;
            st.record(st.resolve(id)?).account.clone()
        };
        let spec = self
            .registry
            .get(&account.provider_id)
            .and_then(|p| p.sub_drives())
            .ok_or_else(|| {
                Error::Unsupported(format!("\"{}\" has no other drives.", account.display_name))
            })?;
        let rc = self.rc().await?;
        let result = rc
            .backend_command(&remote_fs(&account), &spec.list_command)
            .await
            .map_err(|e| {
                let message = e.to_string();
                if crate::status::is_auth_error(&message) {
                    Error::AuthRequired(crate::status::auth_message(&account))
                } else {
                    Error::Rclone(format!(
                        "cannot list the drives of \"{}\": {message}",
                        account.display_name
                    ))
                }
            })?;
        let mut drives: Vec<nuage_core::SubDrive> = result
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                Some(nuage_core::SubDrive {
                    id: d.get("id")?.as_str()?.to_owned(),
                    name: d.get("name")?.as_str()?.to_owned(),
                })
            })
            .collect();
        drives.sort_by_key(|d| d.name.to_lowercase());
        Ok(drives)
    }
}

/// The user's OAuth client of a provider.
impl Inner {
    /// Remotes of the provider's accounts.
    async fn provider_accounts(&self, provider: &str) -> Result<Vec<Account>> {
        if self.registry.get(provider).is_none() {
            return Err(Error::NotFound(format!("Unknown provider \"{provider}\".")));
        }
        let st = self.state.lock().await;
        Ok(st
            .config
            .accounts
            .iter()
            .filter(|r| r.account.provider_id == provider)
            .map(|r| r.account.clone())
            .collect())
    }

    pub(crate) async fn oauth_client(&self, provider: &str) -> Result<nuage_core::OAuthClientInfo> {
        let accounts = self.provider_accounts(provider).await?;
        let rc = self.rc().await?;
        let mut client_id = None;
        for a in &accounts {
            if let Ok(stored) = rc.config_get(&a.remote_name).await
                && let Some(id) = stored
                    .get("client_id")
                    .and_then(Value::as_str)
                    .filter(|v| !v.is_empty())
            {
                client_id = Some(id.to_owned());
                break;
            }
        }
        Ok(nuage_core::OAuthClientInfo {
            client_id,
            accounts: accounts.into_iter().map(|a| a.id).collect(),
        })
    }

    pub(crate) async fn set_oauth_client(
        &self,
        provider_id: &str,
        client_id: &str,
        client_secret: &str,
    ) -> Result<()> {
        let provider = self
            .registry
            .get(provider_id)
            .ok_or_else(|| Error::NotFound(format!("Unknown provider \"{provider_id}\".")))?;
        if !matches!(
            provider.auth(),
            AuthSpec::OAuthBrowser { custom_client } if custom_client != CustomClient::Unsupported
        ) {
            return Err(Error::Unsupported(format!(
                "{} does not use your own sign-in client.",
                provider.display_name()
            )));
        }
        let (client_id, client_secret) = (client_id.trim(), client_secret.trim());
        let suffix_ok = provider
            .client_id_suffix()
            .is_none_or(|s| client_id.len() > s.len() && client_id.ends_with(s));
        if client_id.is_empty() || client_secret.is_empty() || !suffix_ok {
            return Err(Error::InvalidArgument(
                "This client ID or secret is not valid: copy both from the client you created."
                    .to_owned(),
            ));
        }
        let accounts = self.provider_accounts(provider_id).await?;
        let rc = self.rc().await?;
        for account in &accounts {
            let mut parameters = Map::new();
            parameters.insert("client_id".into(), client_id.into());
            parameters.insert("client_secret".into(), client_secret.into());
            parameters.insert("config_refresh_token".into(), "false".into());
            let outcome = rc.config_update(&account.remote_name, &parameters).await?;
            if !outcome.error.is_empty() {
                return Err(Error::Rclone(format!(
                    "Cannot change the client of \"{}\": {}",
                    account.display_name, outcome.error
                )));
            }
            // Its token was issued to the old client.
            let mut st = self.state.lock().await;
            st.runtime(&account.id).auth_error = Some(format!(
                "The sign-in client changed: sign in to {} again (nuage account reconnect \"{}\").",
                account.display_name, account.display_name
            ));
            drop(st);
            self.emit(Event::AuthRequired {
                account: account.id.clone(),
            });
        }
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

#[cfg(test)]
mod connection_tests {
    use super::*;
    use nuage_rclone::RcError;

    fn api(message: &str) -> RcError {
        RcError::Api {
            method: "operations/list".to_owned(),
            status: 500,
            message: message.to_owned(),
        }
    }

    #[test]
    fn messages() {
        let labels = ["user name".to_owned(), "app password".to_owned()];
        let e = connection_error(
            "Nextcloud",
            "",
            &labels,
            &[],
            &api("couldn't list files: 401 Unauthorized"),
        );
        assert_eq!(
            e.to_string(),
            "Nextcloud refused the connection: check the user name and app password."
        );
        let e = connection_error(
            "S3",
            "",
            &["access key ID".to_owned()],
            &["region".to_owned(), "bucket".to_owned()],
            &api("ListBuckets, https response error StatusCode: 403"),
        );
        assert_eq!(
            e.to_string(),
            "S3 refused the connection: check the access key ID, or fill in the region and bucket."
        );
        let e = connection_error("S3", "photos", &[], &[], &api("directory not found"));
        assert!(e.to_string().contains("\"photos\" does not exist"), "{e}");
        let e = connection_error("WebDAV", "", &[], &[], &api("dial tcp: connection refused"));
        assert!(
            e.to_string().starts_with("rclone: Cannot reach WebDAV"),
            "{e}"
        );
    }

    #[test]
    fn labels() {
        assert_eq!(label_in_sentence("Access Key ID"), "access key ID");
        assert_eq!(label_in_sentence("WebDAV Address"), "WebDAV address");
        assert_eq!(label_in_sentence("Bucket (Empty for All)"), "bucket");
    }
}
