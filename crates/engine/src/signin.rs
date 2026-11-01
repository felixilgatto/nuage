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
use nuage_rclone::{ConfigOutcome, ConfigStep, RcClient};

use crate::Inner;

/// rclone's question that leads to the browser sign-in.
pub(crate) const BROWSER_QUESTION: &str = "config_is_local";

/// Where rclone serves the sign-in page.
const SIGN_IN_ADDR: &str = "127.0.0.1:53682";

/// How long a new sign-in waits for the previous one's port.
const PORT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// The account's provider, which ranks rclone's choices.
    provider: String,
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
            "The provider refused the OAuth client. Check the client ID and secret (see https://github.com/felixilgatto/nuage/blob/main/docs/GOOGLE_OAUTH.md)."
                .to_owned(),
        )
    } else if lower.contains("access_denied") {
        Error::AuthRequired("Access was not granted in the browser.".to_owned())
    } else {
        Error::AuthRequired(format!("Sign-in failed: {raw}"))
    }
}

/// The page rclone shows in the browser once the sign-in is done (a Go
/// template: `.OK`, `.Name`, `.Description`, `.Code`, `.HelpURL`), instead of
/// rclone's "All done. Please go back to rclone."
pub(crate) const SIGN_IN_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{ if .OK }}Signed in to Nuage{{ else }}Sign-in failed{{ end }}</title>
<style>
  :root { color-scheme: light dark; }
  body { font-family: system-ui, "Cantarell", sans-serif; margin: 0; min-height: 100vh;
         display: grid; place-items: center; background: Canvas; color: CanvasText; }
  main { max-width: 32rem; padding: 2rem; text-align: center; }
  .mark { font-size: 3rem; line-height: 1; }
  h1 { font-size: 1.6rem; margin: 1rem 0 0.5rem; }
  p { line-height: 1.5; opacity: 0.85; }
  .details { text-align: left; font-size: 0.9rem; opacity: 0.75; overflow-wrap: anywhere; }
</style>
</head>
<body>
<main>
{{ if .OK }}
  <div class="mark">&#10003;</div>
  <h1>You are signed in</h1>
  <p>Nuage received your sign-in. You can close this tab and go back to Nuage to finish.</p>
{{ else }}
  <div class="mark">&#9888;</div>
  <h1>The sign-in did not work</h1>
  <p>Go back to Nuage to try again. If it fails again, check the sign-in client you created.</p>
  <p class="details">{{ .Name }}{{ if .Description }}: {{ .Description }}{{ end }}{{ if .HelpURL }}<br><a href="{{ .HelpURL }}">{{ .HelpURL }}</a>{{ end }}</p>
{{ end }}
</main>
</body>
</html>
"#;

impl Inner {
    /// Fails if a sign-in is already running.
    pub(crate) async fn check_no_sign_in(&self) -> Result<()> {
        if self.sign_in.lock().await.is_some() {
            return Err(Error::Busy(
                "Another sign-in is in progress. Finish or cancel it first.".to_owned(),
            ));
        }
        // A sign-in just cancelled: rclone frees its port a moment later.
        let deadline = Instant::now() + PORT_RELEASE_TIMEOUT;
        while std::net::TcpListener::bind(SIGN_IN_ADDR).is_err() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
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
        provider: &str,
    ) -> Result<SignIn> {
        // rclone takes the page from a file (values are single lines).
        let page = self
            .paths
            .rcd_pid_file()
            .with_file_name("sign-in-page.html");
        std::fs::write(&page, SIGN_IN_PAGE)?;
        let page = page.to_string_lossy().into_owned();
        let started = async {
            let ConfigStep::Job(job) = rc
                .config_continue(
                    remote_name,
                    state,
                    "true",
                    true,
                    // Ephemeral (`config_*`): used for this sign-in only.
                    serde_json::json!({ "config_template_file": page }),
                )
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
            provider: provider.to_owned(),
            job,
            started: Instant::now(),
        });
        tracing::info!(remote = remote_name, "sign-in started");
        Ok(SignIn { session, url })
    }

    /// Answers the questions rclone asks once it has a token (after the
    /// sign-in, or when one was given): ours (`answers`, from the provider's
    /// `config_*` parameters), else rclone's proposals in the provider's
    /// order. When rclone refuses a proposal (e.g. a OneDrive drive whose
    /// root cannot be read), it asks again from an earlier question, and the
    /// next one is tried.
    pub(crate) async fn answer_questions(
        &self,
        rc: &RcClient,
        remote_name: &str,
        mut outcome: ConfigOutcome,
        answers: &[(String, String)],
        provider: &str,
    ) -> Result<()> {
        let provider = self.registry.get(provider);
        let mut tried: Vec<String> = Vec::new();
        let mut proposed = false;
        let mut refused: Option<String> = None;
        for _ in 0..20 {
            if !outcome.error.is_empty() {
                if proposed && !outcome.is_complete() {
                    tracing::warn!(error = %outcome.error, "rclone refused its proposal, trying the next one");
                    refused = Some(outcome.error.clone());
                } else {
                    return Err(sign_in_error(&outcome.error));
                }
            }
            if outcome.is_complete() {
                return Ok(());
            }
            proposed = false;
            let question = outcome.pending_question().unwrap_or_default().to_owned();
            // No question: rclone only moves on.
            let answer = match answers.iter().find(|(k, _)| *k == question) {
                Some((_, answer)) => answer.clone(),
                None if question.is_empty() => String::new(),
                None => {
                    let next = outcome
                        .choices()
                        .into_iter()
                        .filter(|(value, _)| !tried.contains(value))
                        .min_by_key(|(_, help)| {
                            provider.map_or(0, |p| p.choice_rank(&question, help))
                        });
                    let Some((answer, help)) = next else {
                        // Every proposal was refused: say why.
                        return Err(match refused.take() {
                            Some(e) => sign_in_error(&e),
                            None => Error::Rclone(format!(
                                "rclone still asks \"{question}\" after the sign-in."
                            )),
                        });
                    };
                    tracing::info!(%question, %answer, %help, "taking rclone's proposal");
                    tried.push(answer.clone());
                    proposed = true;
                    answer
                }
            };
            tracing::debug!(%question, "answering rclone");
            match rc
                .config_continue(
                    remote_name,
                    &outcome.state,
                    &answer,
                    false,
                    serde_json::json!({}),
                )
                .await
            {
                Ok(ConfigStep::Done(next)) => outcome = next,
                Ok(ConfigStep::Job(_)) => {
                    return Err(Error::Internal("unexpected rc job".to_owned()));
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(Error::Rclone(
            "rclone kept asking questions after the sign-in.".to_owned(),
        ))
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
            let outcome = job.output.clone().unwrap_or_default();
            self.answer_questions(
                &rc,
                &pending.remote_name,
                outcome,
                &pending.answers,
                &pending.provider,
            )
            .await
            .err()
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
        let runtime = st.runtime(&account.id);
        runtime.mount_error = None;
        runtime.auth_error = None;
        // Check the quota (and so the sign-in) again at the next poll.
        runtime.quota_at = None;
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
