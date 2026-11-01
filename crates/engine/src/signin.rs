//! Signing in in a web browser, through rclone's OAuth server.
//!
//! 1. The remote is created (or updated, to reconnect) with
//!    `config_auth_no_browser`: rclone stops at its question "use a web
//!    browser?" (`config_is_local`).
//! 2. Answering "yes" as a background rc job starts rclone's OAuth server on
//!    `127.0.0.1:53682`; `config/oauthstatus` gives the page to open, which
//!    the client shows (and opens if it can). On a server, the user reaches
//!    it through `ssh -L 53682:localhost:53682`.
//! 3. [`Command::SignInWait`] waits for the job: once the user signed in,
//!    the account is saved and mounted (or remounted, to reconnect).
//!
//! rclone runs one OAuth server at a time, so does Nuage. A sign-in nobody
//! waits for is cancelled after [`SIGN_IN_TIMEOUT`].
//!
//! [`Command::SignInWait`]: nuage_core::Command::SignInWait

use std::io::Read;
use std::time::{Duration, Instant};

use nuage_core::{Account, AccountId, Error, Event, Result, SignIn};
use nuage_rclone::{ConfigStep, RcClient};

use crate::Inner;

/// rclone's question that leads to the browser sign-in.
pub(crate) const BROWSER_QUESTION: &str = "config_is_local";

/// How long a sign-in may take.
pub(crate) const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// How long rclone may take to start its OAuth server.
const SERVER_START_TIMEOUT: Duration = Duration::from_secs(15);

/// What a sign-in is for.
#[derive(Debug, Clone)]
pub(crate) enum SignInKind {
    /// A new account, saved and mounted once signed in. Its remote exists
    /// already and is deleted if the sign-in fails.
    Add {
        account: Account,
        mount: bool,
    },
    Reconnect(AccountId),
}

#[derive(Debug, Clone)]
pub(crate) struct PendingSignIn {
    session: String,
    kind: SignInKind,
    remote_name: String,
    /// Answers to rclone's questions after the sign-in (`config_*`
    /// parameters of the provider, e.g. `config_change_team_drive=false`):
    /// rclone does not keep them from the first call.
    answers: Vec<(String, String)>,
    job: u64,
    started: Instant,
}

fn random_session() -> Result<String> {
    let mut buf = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Turns rclone's error into a message that says what to do.
fn sign_in_error(raw: &str) -> Error {
    let lower = raw.to_lowercase();
    if lower.contains("cancelled") || lower.contains("canceled") {
        Error::AuthRequired("Sign-in cancelled.".to_owned())
    } else if lower.contains("address already in use") {
        Error::Busy(
            "Port 53682 is already in use, probably by another rclone sign-in. Close it and try again."
                .to_owned(),
        )
    } else if lower.contains("invalid_client") || lower.contains("unauthorized_client") {
        Error::AuthRequired(
            "The provider refused the OAuth client. Check the client ID and secret (see the README)."
                .to_owned(),
        )
    } else if lower.contains("access_denied") {
        Error::AuthRequired("Access was not granted in the browser.".to_owned())
    } else {
        Error::AuthRequired(format!("Sign-in failed: {raw}"))
    }
}

impl Inner {
    /// Fails if a sign-in is already running.
    pub(crate) async fn check_no_sign_in(&self) -> Result<()> {
        if self.sign_in.lock().await.is_some() {
            return Err(Error::Busy(
                "Another sign-in is in progress. Finish or cancel it first.".to_owned(),
            ));
        }
        Ok(())
    }

    /// Answers rclone's browser question in the background and returns the
    /// page to open. `state` is rclone's state at that question.
    pub(crate) async fn start_sign_in(
        &self,
        rc: &RcClient,
        remote_name: &str,
        state: &str,
        kind: SignInKind,
        answers: Vec<(String, String)>,
    ) -> Result<SignIn> {
        let started = async {
            let ConfigStep::Job(job) = rc
                .config_continue(remote_name, state, "true", true)
                .await
                .map_err(nuage_core::Error::from)?
            else {
                return Err(Error::Internal("expected an rc job".to_owned()));
            };
            let deadline = Instant::now() + SERVER_START_TIMEOUT;
            loop {
                let status = rc.oauth_status().await.map_err(nuage_core::Error::from)?;
                if status.status == "running"
                    && let Some(url) = status.auth_url
                {
                    return Ok((job, url));
                }
                let progress = rc.config_job(job).await.map_err(nuage_core::Error::from)?;
                if progress.finished {
                    return Err(sign_in_error(&progress.error));
                }
                if Instant::now() > deadline {
                    let _ = rc.oauth_stop().await;
                    return Err(Error::Rclone(
                        "rclone did not start its sign-in server.".to_owned(),
                    ));
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        .await;
        let (job, url) = match started {
            Ok(v) => v,
            Err(e) => {
                self.discard_sign_in(rc, &kind, remote_name).await;
                return Err(e);
            }
        };
        let session = random_session()?;
        *self.sign_in.lock().await = Some(PendingSignIn {
            session: session.clone(),
            kind,
            remote_name: remote_name.to_owned(),
            answers,
            job,
            started: Instant::now(),
        });
        tracing::info!(remote = remote_name, "sign-in started");
        Ok(SignIn { session, url })
    }

    /// Deletes what a failed or cancelled sign-in created.
    async fn discard_sign_in(&self, rc: &RcClient, kind: &SignInKind, remote_name: &str) {
        if let SignInKind::Add { account, .. } = kind {
            if let Err(e) = rc.config_delete(remote_name).await {
                tracing::warn!(remote = remote_name, error = %e, "cannot delete the remote");
            }
            let _ = std::fs::remove_dir(&account.mount_point);
        }
    }

    fn pending_matches(pending: &Option<PendingSignIn>, session: &str) -> Result<PendingSignIn> {
        match pending {
            Some(p) if p.session == session => Ok(p.clone()),
            _ => Err(Error::NotFound(
                "This sign-in is not in progress any more.".to_owned(),
            )),
        }
    }

    /// Waits for the user to sign in, then finishes adding or reconnecting.
    pub(crate) async fn wait_sign_in(&self, session: &str) -> Result<Account> {
        let rc = self.rc().await?;
        let pending = Self::pending_matches(&*self.sign_in.lock().await, session)?;
        let job = loop {
            let progress = rc
                .config_job(pending.job)
                .await
                .map_err(nuage_core::Error::from)?;
            if progress.finished {
                break progress;
            }
            if pending.started.elapsed() > SIGN_IN_TIMEOUT {
                self.cancel_sign_in(session).await?;
                return Err(Error::AuthRequired(
                    "The sign-in took too long and was cancelled. Try again.".to_owned(),
                ));
            }
            // Cancelled from another connection.
            Self::pending_matches(&*self.sign_in.lock().await, session)?;
            tokio::time::sleep(Duration::from_millis(500)).await;
        };
        {
            let mut current = self.sign_in.lock().await;
            if current.as_ref().is_some_and(|p| p.session == session) {
                *current = None;
            }
        }

        let failure = if !job.error.is_empty() {
            Some(sign_in_error(&job.error))
        } else {
            let mut outcome = job.output.clone().unwrap_or_default();
            let mut failure = None;
            // Answer the questions rclone asks after the sign-in.
            for _ in 0..10 {
                if !outcome.error.is_empty() {
                    failure = Some(sign_in_error(&outcome.error));
                    break;
                }
                if outcome.is_complete() {
                    break;
                }
                let question = outcome.pending_question().unwrap_or_default().to_owned();
                let Some((_, answer)) = pending.answers.iter().find(|(k, _)| *k == question) else {
                    failure = Some(Error::Rclone(format!(
                        "rclone still asks \"{question}\" after the sign-in."
                    )));
                    break;
                };
                tracing::debug!(%question, "answering rclone");
                match rc
                    .config_continue(&pending.remote_name, &outcome.state, answer, false)
                    .await
                {
                    Ok(ConfigStep::Done(next)) => outcome = next,
                    Ok(ConfigStep::Job(_)) => {
                        failure = Some(Error::Internal("unexpected rc job".to_owned()));
                        break;
                    }
                    Err(e) => {
                        failure = Some(e.into());
                        break;
                    }
                }
            }
            if failure.is_none() && !outcome.is_complete() {
                failure = Some(Error::Rclone(
                    "rclone kept asking questions after the sign-in.".to_owned(),
                ));
            }
            failure
        };
        if let Some(e) = failure {
            tracing::warn!(remote = %pending.remote_name, error = %e, "sign-in failed");
            self.discard_sign_in(&rc, &pending.kind, &pending.remote_name)
                .await;
            return Err(e);
        }
        tracing::info!(remote = %pending.remote_name, "signed in");

        match pending.kind {
            SignInKind::Add { account, mount } => {
                let mut st = self.state.lock().await;
                let taken = st.config.accounts.iter().any(|r| {
                    r.account
                        .display_name
                        .eq_ignore_ascii_case(&account.display_name)
                });
                if taken {
                    let _ = rc.config_delete(&pending.remote_name).await;
                    return Err(Error::AlreadyExists(format!(
                        "An account named \"{}\" was added meanwhile.",
                        account.display_name
                    )));
                }
                self.finish_add(&mut st, &rc, account, mount).await
            }
            SignInKind::Reconnect(id) => self.finish_reconnect(&id).await,
        }
    }

    /// Cancels the sign-in `session`.
    pub(crate) async fn cancel_sign_in(&self, session: &str) -> Result<()> {
        let pending = {
            let mut current = self.sign_in.lock().await;
            let pending = Self::pending_matches(&current, session)?;
            *current = None;
            pending
        };
        let rc = self.rc().await?;
        let _ = rc.oauth_stop().await;
        self.discard_sign_in(&rc, &pending.kind, &pending.remote_name)
            .await;
        tracing::info!(remote = %pending.remote_name, "sign-in cancelled");
        Ok(())
    }

    /// Cancels a sign-in nobody finished in time (called every tick).
    pub(crate) async fn expire_sign_in(&self) {
        let expired = {
            let current = self.sign_in.lock().await;
            current
                .as_ref()
                .filter(|p| p.started.elapsed() > SIGN_IN_TIMEOUT + Duration::from_secs(30))
                .map(|p| p.session.clone())
        };
        if let Some(session) = expired {
            tracing::info!("cancelling an abandoned sign-in");
            let _ = self.cancel_sign_in(&session).await;
        }
    }

    /// After a new token: remount so that rclone uses it.
    pub(crate) async fn finish_reconnect(&self, id: &AccountId) -> Result<Account> {
        let mut st = self.state.lock().await;
        let i = st.resolve(id)?;
        let account = st.record(i).account.clone();
        if st.is_mounted(&account.id) {
            self.unmount_locked(&mut st, &account).await?;
        }
        st.runtime(&account.id).mount_error = None;
        if st.record(i).auto_mount {
            self.mount_locked(&mut st, i).await?;
        }
        self.emit(Event::AccountUpdated {
            account: account.clone(),
        });
        Ok(account)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors() {
        assert!(matches!(
            sign_in_error("config failed to refresh token: oauth authentication was cancelled"),
            Error::AuthRequired(m) if m == "Sign-in cancelled."
        ));
        assert!(matches!(
            sign_in_error("listen tcp 127.0.0.1:53682: bind: address already in use"),
            Error::Busy(_)
        ));
        assert!(
            sign_in_error("oauth2: \"invalid_client\"")
                .to_string()
                .contains("client ID")
        );
    }
}
