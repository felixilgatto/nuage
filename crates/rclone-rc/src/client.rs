use std::time::Duration;

use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::types::{
    About, ConfigJob, ConfigOutcome, ConfigStep, CoreVersion, FsInfo, JobStatus, ListItem,
    MountInfo, MountRequest, OAuthStatus, QueueItem, VfsStats,
};

/// Timeout of calls that should answer quickly (stats, queue, version).
const SHORT: Duration = Duration::from_secs(10);
/// Timeout of calls that talk to the cloud provider.
const LONG: Duration = Duration::from_secs(120);
/// How often a running `_async` job is polled.
const JOB_POLL: Duration = Duration::from_millis(250);

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
        let mut base = Url::parse(base)?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .build()?;
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
        self.call_with_timeout(method, params, LONG).await
    }

    /// [`call`](Self::call) with an explicit timeout.
    pub async fn call_with_timeout<P, R>(
        &self,
        method: &str,
        params: &P,
        timeout: Duration,
    ) -> Result<R, RcError>
    where
        P: Serialize + ?Sized,
        R: DeserializeOwned,
    {
        let url = self.base.join(method)?;
        // Parameters are never logged: they may contain tokens and secrets.
        tracing::trace!(%method, "rc call");
        let resp = self
            .http
            .post(url)
            .basic_auth(&self.user, Some(&self.pass))
            .timeout(timeout)
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
        self.call_with_timeout("core/version", &json!({}), SHORT)
            .await
    }

    /// `core/quit`: stops rcd. Pending VFS writes stay in the cache.
    pub async fn core_quit(&self) -> Result<(), RcError> {
        self.call_with_timeout::<_, IgnoredAny>("core/quit", &json!({}), SHORT)
            .await
            .map(drop)
    }

    // Remotes

    /// `config/create` with `nonInteractive` and `obscure`: creates a remote
    /// from `parameters` without asking any question.
    ///
    /// rclone saves the remote before the questions end: when the outcome is
    /// not complete, the caller deletes it.
    pub async fn config_create(
        &self,
        name: &str,
        backend: &str,
        parameters: &serde_json::Map<String, Value>,
    ) -> Result<ConfigOutcome, RcError> {
        self.call(
            "config/create",
            &json!({
                "name": name,
                "type": backend,
                "parameters": parameters,
                "opt": { "nonInteractive": true, "obscure": true },
            }),
        )
        .await
    }

    /// `config/update` with `continue`: answers the pending question of a
    /// remote's configuration. With `background`, runs as an rc job (the
    /// OAuth step blocks until the user signs in) and returns its id.
    pub async fn config_continue(
        &self,
        name: &str,
        state: &str,
        result: &str,
        background: bool,
    ) -> Result<ConfigStep, RcError> {
        let params = json!({
            "name": name,
            "parameters": {},
            "opt": { "nonInteractive": true, "continue": true, "state": state, "result": result },
            "_async": background,
        });
        if background {
            #[derive(Deserialize)]
            struct Job {
                jobid: u64,
            }
            let job: Job = self
                .call_with_timeout("config/update", &params, SHORT)
                .await?;
            Ok(ConfigStep::Job(job.jobid))
        } else {
            Ok(ConfigStep::Done(self.call("config/update", &params).await?))
        }
    }

    /// `config/update` with plain parameters (e.g. a new token).
    pub async fn config_update(
        &self,
        name: &str,
        parameters: &serde_json::Map<String, Value>,
    ) -> Result<ConfigOutcome, RcError> {
        self.call(
            "config/update",
            &json!({
                "name": name,
                "parameters": parameters,
                "opt": { "nonInteractive": true, "obscure": true },
            }),
        )
        .await
    }

    /// `config/get`: the parameters of a remote, as stored.
    pub async fn config_get(&self, name: &str) -> Result<serde_json::Map<String, Value>, RcError> {
        self.call_with_timeout("config/get", &json!({ "name": name }), SHORT)
            .await
    }

    /// `config/oauthstatus`: the sign-in URL while rclone's OAuth server runs.
    pub async fn oauth_status(&self) -> Result<OAuthStatus, RcError> {
        self.call_with_timeout("config/oauthstatus", &json!({}), SHORT)
            .await
    }

    /// `config/oauthstop`: cancels a running OAuth sign-in.
    pub async fn oauth_stop(&self) -> Result<(), RcError> {
        self.call_with_timeout::<_, IgnoredAny>("config/oauthstop", &json!({}), SHORT)
            .await
            .map(drop)
    }

    /// `job/status` of a configuration job started by
    /// [`config_continue`](Self::config_continue).
    pub async fn config_job(&self, id: u64) -> Result<ConfigJob, RcError> {
        self.call_with_timeout("job/status", &json!({ "jobid": id }), SHORT)
            .await
    }

    /// `config/delete`.
    pub async fn config_delete(&self, name: &str) -> Result<(), RcError> {
        self.call_with_timeout::<_, IgnoredAny>("config/delete", &json!({ "name": name }), SHORT)
            .await
            .map(drop)
    }

    /// `config/listremotes`.
    pub async fn config_listremotes(&self) -> Result<Vec<String>, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            remotes: Vec<String>,
        }
        let a: Answer = self
            .call_with_timeout("config/listremotes", &json!({}), SHORT)
            .await?;
        Ok(a.remotes)
    }

    // Backend information

    /// `operations/fsinfo`: backend features and hashes.
    pub async fn fsinfo(&self, fs: &str) -> Result<FsInfo, RcError> {
        self.call("operations/fsinfo", &json!({ "fs": fs })).await
    }

    /// `operations/about`: quota. Check the `About` feature first.
    pub async fn about(&self, fs: &str) -> Result<About, RcError> {
        self.call("operations/about", &json!({ "fs": fs })).await
    }

    // Mounts

    /// `mount/mount`.
    pub async fn mount(&self, request: &MountRequest) -> Result<(), RcError> {
        self.call::<_, IgnoredAny>("mount/mount", request)
            .await
            .map(drop)
    }

    /// `mount/unmount`.
    pub async fn unmount(&self, mount_point: &str) -> Result<(), RcError> {
        self.call::<_, IgnoredAny>("mount/unmount", &json!({ "mountPoint": mount_point }))
            .await
            .map(drop)
    }

    /// `mount/listmounts`.
    pub async fn list_mounts(&self) -> Result<Vec<MountInfo>, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(rename = "mountPoints", default)]
            mount_points: Vec<MountInfo>,
        }
        let a: Answer = self
            .call_with_timeout("mount/listmounts", &json!({}), SHORT)
            .await?;
        Ok(a.mount_points)
    }

    // VFS

    /// `vfs/stats` of one VFS (`fs` is mandatory once several are active).
    pub async fn vfs_stats(&self, fs: &str) -> Result<VfsStats, RcError> {
        self.call_with_timeout("vfs/stats", &json!({ "fs": fs }), SHORT)
            .await
    }

    /// `vfs/queue`: the upload queue of one VFS.
    pub async fn vfs_queue(&self, fs: &str) -> Result<Vec<QueueItem>, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            queue: Vec<QueueItem>,
        }
        let a: Answer = self
            .call_with_timeout("vfs/queue", &json!({ "fs": fs }), SHORT)
            .await?;
        Ok(a.queue)
    }

    /// `vfs/queue-set-expiry`: `expiry` seconds from now (negative: upload
    /// as soon as possible).
    pub async fn vfs_queue_set_expiry(
        &self,
        fs: &str,
        id: u64,
        expiry: f64,
    ) -> Result<(), RcError> {
        self.call_with_timeout::<_, IgnoredAny>(
            "vfs/queue-set-expiry",
            &json!({ "fs": fs, "id": id, "expiry": expiry }),
            SHORT,
        )
        .await
        .map(drop)
    }

    /// `vfs/forget` then `vfs/refresh` of the root: the next listings come
    /// from the provider instead of the directory cache.
    pub async fn vfs_refresh(&self, fs: &str) -> Result<(), RcError> {
        self.call::<_, IgnoredAny>("vfs/forget", &json!({ "fs": fs }))
            .await?;
        self.call::<_, IgnoredAny>("vfs/refresh", &json!({ "fs": fs }))
            .await
            .map(drop)
    }
}

// Files. `fs` is a remote (`"name:"`) or a local folder (`"/home/me"`);
// `remote` is a path inside it, without a leading slash.
impl RcClient {
    /// `operations/list` of one folder (not recursive).
    pub async fn list(&self, fs: &str, remote: &str) -> Result<Vec<ListItem>, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            list: Vec<ListItem>,
        }
        let a: Answer = self
            .call("operations/list", &json!({ "fs": fs, "remote": remote }))
            .await?;
        Ok(a.list)
    }

    /// `operations/stat`: `None` when nothing is at `remote`.
    pub async fn stat(&self, fs: &str, remote: &str) -> Result<Option<ListItem>, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            #[serde(default)]
            item: Option<ListItem>,
        }
        let a: Answer = self
            .call("operations/stat", &json!({ "fs": fs, "remote": remote }))
            .await?;
        Ok(a.item)
    }

    /// `operations/mkdir`.
    pub async fn mkdir(&self, fs: &str, remote: &str) -> Result<(), RcError> {
        self.call::<_, IgnoredAny>("operations/mkdir", &json!({ "fs": fs, "remote": remote }))
            .await
            .map(drop)
    }

    /// `operations/rmdir`: removes an empty folder.
    pub async fn rmdir(&self, fs: &str, remote: &str) -> Result<(), RcError> {
        self.call::<_, IgnoredAny>("operations/rmdir", &json!({ "fs": fs, "remote": remote }))
            .await
            .map(drop)
    }

    /// `operations/deletefile`.
    pub async fn delete_file(&self, fs: &str, remote: &str) -> Result<(), RcError> {
        self.job(
            "operations/deletefile",
            json!({ "fs": fs, "remote": remote }),
        )
        .await
        .map(drop)
    }

    /// `operations/purge`: removes a folder and everything in it.
    pub async fn purge(&self, fs: &str, remote: &str) -> Result<(), RcError> {
        self.job("operations/purge", json!({ "fs": fs, "remote": remote }))
            .await
            .map(drop)
    }

    /// `operations/copyfile` or `operations/movefile` (`moving`).
    pub async fn transfer_file(
        &self,
        (src_fs, src): (&str, &str),
        (dst_fs, dst): (&str, &str),
        moving: bool,
    ) -> Result<(), RcError> {
        let method = if moving {
            "operations/movefile"
        } else {
            "operations/copyfile"
        };
        let params = json!({
            "srcFs": src_fs, "srcRemote": src, "dstFs": dst_fs, "dstRemote": dst,
        });
        self.job(method, params).await.map(drop)
    }

    /// `sync/copy` or `sync/move` (`moving`) of a whole folder; both fs
    /// strings name folders (`"remote:a/b"`).
    pub async fn transfer_dir(
        &self,
        src_fs: &str,
        dst_fs: &str,
        moving: bool,
    ) -> Result<(), RcError> {
        let (method, params) = if moving {
            (
                "sync/move",
                json!({ "srcFs": src_fs, "dstFs": dst_fs, "deleteEmptySrcDirs": true,
                        "createEmptySrcDirs": true }),
            )
        } else {
            (
                "sync/copy",
                json!({ "srcFs": src_fs, "dstFs": dst_fs, "createEmptySrcDirs": true }),
            )
        };
        self.job(method, params).await.map(drop)
    }

    /// `operations/publiclink`: a share link.
    pub async fn public_link(&self, fs: &str, remote: &str) -> Result<String, RcError> {
        #[derive(Deserialize)]
        struct Answer {
            url: String,
        }
        let a: Answer = self
            .call(
                "operations/publiclink",
                &json!({ "fs": fs, "remote": remote }),
            )
            .await?;
        Ok(a.url)
    }

    /// `vfs/forget` of some folders of a mounted fs: the mount lists them
    /// again from the provider. Paths are relative to the fs root.
    pub async fn vfs_forget_dirs(&self, fs: &str, dirs: &[String]) -> Result<(), RcError> {
        let mut params = serde_json::Map::new();
        params.insert("fs".into(), fs.into());
        for (i, p) in dirs.iter().enumerate() {
            let key = if i == 0 {
                "dir".to_owned()
            } else {
                format!("dir{i}")
            };
            params.insert(key, p.as_str().into());
        }
        self.call_with_timeout::<_, IgnoredAny>("vfs/forget", &Value::Object(params), SHORT)
            .await
            .map(drop)
    }

    /// Runs `method` as an `_async` job and waits for its end: transfers
    /// can outlast any HTTP timeout.
    pub async fn job(&self, method: &str, mut params: Value) -> Result<Value, RcError> {
        #[derive(Deserialize)]
        struct Started {
            jobid: u64,
        }
        if let Value::Object(map) = &mut params {
            map.insert("_async".into(), true.into());
        }
        let started: Started = self.call_with_timeout(method, &params, SHORT).await?;
        loop {
            let status: JobStatus = self
                .call_with_timeout("job/status", &json!({ "jobid": started.jobid }), SHORT)
                .await?;
            if status.finished {
                if status.success {
                    return Ok(status.output.unwrap_or(Value::Null));
                }
                return Err(RcError::Api {
                    method: method.to_owned(),
                    status: 500,
                    message: status.error,
                });
            }
            tokio::time::sleep(JOB_POLL).await;
        }
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
