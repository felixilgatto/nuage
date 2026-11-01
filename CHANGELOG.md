# Changelog

Notable changes of Nuage. Versions follow [semantic versioning](https://semver.org/).

## [Unreleased]

- Folders mount after a restart even when the keyring is slow to answer at
  login (a session that logs in automatically, or a desktop portal still
  starting): Nuage waits for it instead of stopping. Meanwhile, the tray shows
  "Waiting for the keyring" with a key, not a problem; it becomes a problem
  after 3 minutes. The window starts the Nuage service again if it crashes.
- OneDrive: the drive named "OneDrive" is tried first; adding an account on a
  server with `--token-file` answers the same questions as a sign-in.
- Removing an account no longer fills the log with errors about its cache.
- The OneDrive Personal Vault, which cannot be opened, is documented.
- After an update installed while Nuage runs, folders still mount, and Nuage
  offers to restart into the new version.
- Leaving the folder screen without starting the sync no longer leaves a
  half-added account; going back is locked while the folder is set up.
- After adding an account, the window stays open on the accounts list, with
  "Open Folder"; "Start Sync" shows that it is working.
- How to uninstall, in the README.
- Adding a OneDrive account no longer stops at "rclone still asks config_driveid": Nuage takes the drive rclone proposes, and the next one if Microsoft refuses it.
- OneDrive (personal): sign in in the browser, no client to create.
- Google shared drives, each as an account of its own, added from a Google
  account without signing in again.
- Offline mode: keep files and folders on this computer ("Keep Offline" in
  the browser, `nuage pin`). They open and can be edited without a
  connection, also after a restart; changes upload when it comes back, and a
  file changed in two places keeps both versions.
- A warning when the items kept offline take more than a chosen space
  (`offline-warning`).
- Offline, the other items stay listed and fail to open at once, without
  blocking file managers.
- After signing in, the browser shows a Nuage page and Nuage comes back to
  the front.

## [0.1.0] - 2026-11-01

First version.

- Google Drive and local folders, mounted with rclone in a folder of your
  home: files download when opened, changes upload in the background.
- Desktop application: setup in a few screens, main window, system tray
  icon, file browser, preferences, notifications.
- `nuage` command line and `nuaged` service for servers, without any
  prompt: accounts, mounts, pause and resume, file commands, exclusions,
  settings.
- Exclusions: folders and patterns kept out of the sync, temporary files
  by default.
- Packages: Flatpak, static tarball and `.deb`.
