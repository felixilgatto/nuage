use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

/// Errors of the rc client.
#[derive(Debug, thiserror::Error)]
pub enum RcError {
    #[error("invalid rc URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("cannot reach rclone: {0}")]
    Http(#[from] reqwest::Error),
    /// rclone answered with an error status; `message` is its `error` field.
    #[error("rclone {method} failed ({status}): {message}")]
    Api {
        method: String,
        status: u16,
        message: String,
    },
}

impl From<RcError> for nuage_core::Error {
    fn from(e: RcError) -> Self {
        nuage_core::Error::Rclone(e.to_string())
    }
}

/// Answer of `core/version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreVersion {
    /// e.g. `"v1.71.0"`.
    pub version: String,
    /// e.g. `[1, 71, 0]`.
    #[serde(default)]
    pub decomposed: Vec<u32>,
    #[serde(rename = "isGit", default)]
    pub is_git: bool,
    #[serde(rename = "isBeta", default)]
    pub is_beta: bool,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(rename = "goVersion", default)]
    pub go_version: String,
}

/// Client of one `rclone rcd` instance.
///
/// rcd listens on 127.0.0.1 with a random port and random credentials (never
/// `--rc-no-auth`). Every call is a `POST <base>/<method>` with a JSON body.
#[derive(Debug, Clone)]
pub struct RcClient {
    http: reqwest::Client,
    base: Url,
    user: String,
    pass: String,
}

#[derive(Deserialize)]
struct ApiError {
    error: String,
}

impl RcClient {
    /// Creates a client for the rcd at `base` (e.g. `http://127.0.0.1:5572/`).
    pub fn new(base: &str, user: &str, pass: &str) -> Result<Self, RcError> {
        // reqwest is built with rustls without a default crypto provider;
        // install `ring` once per process. Failure means another provider is
        // already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut base = Url::parse(base)?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder().no_proxy().build()?;
        Ok(Self {
            http,
            base,
            user: user.to_owned(),
            pass: pass.to_owned(),
        })
    }

    /// Calls an rc method with JSON parameters and decodes its JSON answer.
    pub async fn call<P, R>(&self, method: &str, params: &P) -> Result<R, RcError>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = self.base.join(method)?;
        tracing::debug!(%method, "rc call");
        let resp = self
            .http
            .post(url)
            .basic_auth(&self.user, Some(&self.pass))
            .json(params)
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp.json().await?)
        } else {
            let message = match resp.json::<ApiError>().await {
                Ok(e) => e.error,
                Err(_) => status
                    .canonical_reason()
                    .unwrap_or("unknown error")
                    .to_owned(),
            };
            Err(RcError::Api {
                method: method.to_owned(),
                status: status.as_u16(),
                message,
            })
        }
    }

    /// `core/version`: health check and version of the running rclone.
    pub async fn core_version(&self) -> Result<CoreVersion, RcError> {
        self.call("core/version", &serde_json::json!({})).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serves one HTTP request with a canned answer and returns the request.
    async fn one_shot_server(
        status: &'static str,
        body: &'static str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Read headers, then the body announced by Content-Length.
            loop {
                let n = sock.read(&mut chunk).await.unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text[..end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf).to_string()
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn core_version() {
        let (url, server) = one_shot_server(
            "200 OK",
            r#"{"arch":"amd64","decomposed":[1,71,0],"goTags":"none","goVersion":"go1.25","isBeta":false,"isGit":false,"linking":"static","os":"linux","version":"v1.71.0"}"#,
        )
        .await;
        let client = RcClient::new(&url, "user", "secret").unwrap();
        let v = client.core_version().await.unwrap();
        assert_eq!(v.version, "v1.71.0");
        assert_eq!(v.decomposed, vec![1, 71, 0]);
        assert_eq!(v.os, "linux");

        let request = server.await.unwrap();
        assert!(
            request.starts_with("POST /core/version HTTP/1.1"),
            "{request}"
        );
        // "user:secret" in base64.
        assert!(request.contains("dXNlcjpzZWNyZXQ="), "{request}");
    }

    #[tokio::test]
    async fn api_error() {
        let (url, _server) = one_shot_server(
            "500 Internal Server Error",
            r#"{"error":"couldn't find method","input":{},"path":"core/version","status":500}"#,
        )
        .await;
        let client = RcClient::new(&url, "u", "p").unwrap();
        match client.core_version().await.unwrap_err() {
            RcError::Api {
                status, message, ..
            } => {
                assert_eq!(status, 500);
                assert_eq!(message, "couldn't find method");
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}
