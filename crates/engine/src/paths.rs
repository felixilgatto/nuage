use std::path::{Path, PathBuf};

use nuage_core::{Error, Result};

/// Where Nuage keeps its files.
///
/// | Content | Native | Flatpak (`~/.var/app/<id>/…`) |
/// |---|---|---|
/// | app configuration (`config.toml`) | `~/.config/nuage/` | `config/nuage/` |
/// | `rclone.conf` | `~/.config/nuage/rclone.conf` | `config/rclone/rclone.conf` |
/// | VFS cache (`--cache-dir`) | `~/.cache/nuage/rclone` | `cache/rclone` |
/// | hidden mounts, rcd PID file | `~/.local/share/nuage/` | `data/` |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home: PathBuf,
    /// Directory of `config.toml`.
    pub config_dir: PathBuf,
    pub rclone_config: PathBuf,
    /// rclone `--cache-dir`.
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    /// Running inside the Flatpak sandbox.
    pub in_flatpak: bool,
}

impl Paths {
    /// Paths from the XDG environment variables of the current process.
    pub fn from_env() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .filter(|h| !h.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| Error::Internal("HOME is not set".to_owned()))?;
        let xdg = |var: &str, default: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        let config = xdg("XDG_CONFIG_HOME", ".config");
        let cache = xdg("XDG_CACHE_HOME", ".cache");
        let data = xdg("XDG_DATA_HOME", ".local/share");
        let in_flatpak =
            std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
        Ok(if in_flatpak {
            // Inside the sandbox the XDG directories are already private to
            // the application (`~/.var/app/<id>/…`).
            Self {
                config_dir: config.join("nuage"),
                rclone_config: config.join("rclone").join("rclone.conf"),
                cache_dir: cache.join("rclone"),
                data_dir: data,
                home,
                in_flatpak,
            }
        } else {
            Self {
                config_dir: config.join("nuage"),
                rclone_config: config.join("nuage").join("rclone.conf"),
                cache_dir: cache.join("nuage").join("rclone"),
                data_dir: data.join("nuage"),
                home,
                in_flatpak,
            }
        })
    }

    /// Every path under `root`, for tests.
    pub fn under(root: &Path) -> Self {
        Self {
            home: root.join("home"),
            config_dir: root.join("config"),
            rclone_config: root.join("config").join("rclone.conf"),
            cache_dir: root.join("cache"),
            data_dir: root.join("data"),
            in_flatpak: false,
        }
    }

    pub fn app_config(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    /// Parent of the hidden rclone mounts used under `nuage-fs`.
    pub fn hidden_mounts(&self) -> PathBuf {
        self.data_dir.join("mounts")
    }

    /// Excluded items created locally in an account's folder.
    pub fn local_only(&self, account: &nuage_core::AccountId) -> PathBuf {
        self.data_dir.join("local-only").join(account.as_str())
    }

    pub fn rcd_pid_file(&self) -> PathBuf {
        self.data_dir.join("rcd.pid")
    }

    /// Folder an account is mounted at when the user does not choose one.
    pub fn default_mount_point(&self, account_name: &str) -> PathBuf {
        let folder: String = account_name
            .chars()
            .map(|c| if c == '/' || c == '\0' { '-' } else { c })
            .collect();
        self.home.join("Cloud").join(folder)
    }

    /// VFS cache directories of a remote (data and metadata).
    pub fn remote_cache_dirs(&self, remote_name: &str) -> [PathBuf; 2] {
        [
            self.cache_dir.join("vfs").join(remote_name),
            self.cache_dir.join("vfsMeta").join(remote_name),
        ]
    }

    /// Directories a mount point must stay out of.
    pub fn private_dirs(&self) -> [&Path; 3] {
        [&self.config_dir, &self.cache_dir, &self.data_dir]
    }
}
