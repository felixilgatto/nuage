//! Updates of Nuage itself, through the Flatpak update monitor portal.

/// What the Flatpak update monitor reports about Nuage itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateEvent {
    /// A newer version is available in the repository, not installed yet.
    Available,
    /// A newer version is installed: this instance and its daemon keep
    /// running the old one until restarted.
    Installed,
    /// Installing the update failed, with the portal's message.
    Failed(String),
}

/// What a commit triple of `UpdateAvailable` means.
fn classify(running: &str, local: &str, remote: &str) -> Option<UpdateEvent> {
    if !local.is_empty() && local != running {
        Some(UpdateEvent::Installed)
    } else if !remote.is_empty() && remote != running {
        Some(UpdateEvent::Available)
    } else {
        None
    }
}

/// Whether this program runs in the Flatpak sandbox.
pub fn in_flatpak() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists()
}

/// Runs the Flatpak update monitor until it closes: reports on `tx`, and
/// installs the available update on each message of `install`. Must run
/// inside a tokio runtime.
pub async fn monitor(
    tx: async_channel::Sender<UpdateEvent>,
    install: async_channel::Receiver<()>,
) -> Result<(), String> {
    use ashpd::flatpak::{Flatpak, UpdateStatus};
    use futures_util::StreamExt;
    let message = |e: ashpd::Error| e.to_string();
    let flatpak = Flatpak::new().await.map_err(message)?;
    let monitor = flatpak.create_update_monitor().await.map_err(message)?;
    let mut available = monitor.receive_update_available().await.map_err(message)?;
    let mut progress = monitor.receive_progress().await.map_err(message)?;
    loop {
        tokio::select! {
            info = available.next() => {
                let Some(info) = info else { break };
                tracing::info!(
                    running = info.running_commit(),
                    local = info.local_commit(),
                    remote = info.remote_commit(),
                    "update monitor"
                );
                let event = classify(
                    info.running_commit(),
                    info.local_commit(),
                    info.remote_commit(),
                );
                if let Some(event) = event {
                    let _ = tx.send(event).await;
                }
            }
            p = progress.next() => {
                let Some(p) = p else { break };
                let event = match p.status {
                    Some(UpdateStatus::Done) => UpdateEvent::Installed,
                    Some(UpdateStatus::Failed) => UpdateEvent::Failed(
                        p.error_message.or(p.error).unwrap_or_default(),
                    ),
                    _ => continue,
                };
                let _ = tx.send(event).await;
            }
            request = install.recv() => {
                if request.is_err() {
                    break;
                }
                if let Err(e) = monitor.update(None).await {
                    let _ = tx.send(UpdateEvent::Failed(e.to_string())).await;
                }
            }
        }
    }
    Err("the update monitor closed".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{UpdateEvent, classify};

    #[test]
    fn update_states() {
        // Nothing new.
        assert_eq!(classify("a", "a", "a"), None);
        assert_eq!(classify("a", "a", ""), None);
        // In the repository only.
        assert_eq!(classify("a", "a", "b"), Some(UpdateEvent::Available));
        // Installed while running, whatever the repository has.
        assert_eq!(classify("a", "b", "b"), Some(UpdateEvent::Installed));
        assert_eq!(classify("a", "b", "c"), Some(UpdateEvent::Installed));
    }
}
