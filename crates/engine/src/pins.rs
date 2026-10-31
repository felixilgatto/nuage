use nuage_core::{AccountId, Error, Pin, PinKind, RemotePath, Result};

/// Offline pinning (v1.1), backed by `nuage-fs`.
#[derive(Debug, Default)]
pub struct PinManager {}

impl PinManager {
    pub async fn add(&self, _path: &RemotePath, _kind: PinKind) -> Result<()> {
        Err(Error::not_implemented("offline pinning (planned for v1.1)"))
    }

    pub async fn remove(&self, _path: &RemotePath) -> Result<()> {
        Err(Error::not_implemented("offline pinning (planned for v1.1)"))
    }

    pub async fn list(&self, _account: Option<&AccountId>) -> Result<Vec<Pin>> {
        Err(Error::not_implemented("offline pinning (planned for v1.1)"))
    }
}
