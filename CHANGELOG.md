# Changelog

Notable changes of Nuage. Versions follow [semantic versioning](https://semver.org/).

## [Unreleased]

- After adding an account, the window stays open on the accounts list, with
  "Open Folder"; "Start Sync" shows that it is working.
- How to uninstall, in the README.
- Adding a OneDrive account no longer stops at "rclone still asks config_driveid": Nuage takes the drive rclone proposes.
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
