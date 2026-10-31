use std::path::Path;

use nuage_core::{AccountId, Entry, Error, RemotePath, Result};

/// File operations through the rc API, independent of the mount: they work
/// even when the account is unmounted.
#[derive(Debug, Default)]
pub struct FileService {}

impl FileService {
    pub async fn list(&self, _path: &RemotePath) -> Result<Vec<Entry>> {
        Err(Error::not_implemented("listing files"))
    }

    pub async fn mkdir(&self, _path: &RemotePath) -> Result<()> {
        Err(Error::not_implemented("creating a folder"))
    }

    pub async fn move_to(&self, _from: &RemotePath, _to: &RemotePath) -> Result<()> {
        Err(Error::not_implemented("moving files"))
    }

    pub async fn copy_to(&self, _from: &RemotePath, _to: &RemotePath) -> Result<()> {
        Err(Error::not_implemented("copying files"))
    }

    pub async fn delete(&self, _path: &RemotePath, _recursive: bool) -> Result<()> {
        Err(Error::not_implemented("deleting files"))
    }

    pub async fn upload(&self, _local: &Path, _to: &RemotePath) -> Result<()> {
        Err(Error::not_implemented("uploading files"))
    }

    pub async fn download(&self, _from: &RemotePath, _local: &Path) -> Result<()> {
        Err(Error::not_implemented("downloading files"))
    }

    pub async fn public_link(&self, _path: &RemotePath) -> Result<String> {
        Err(Error::not_implemented("creating a share link"))
    }

    /// `vfs/refresh` for one account or all of them.
    pub async fn refresh(&self, _account: Option<&AccountId>) -> Result<()> {
        Err(Error::not_implemented("refreshing"))
    }
}
