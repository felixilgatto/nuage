# Changelog

Notable changes of Nuage. Versions follow [semantic versioning](https://semver.org/).

## [Unreleased]

## [0.2.3] - 2026-11-11

- Files kept offline open again instantly: Nuage keeps them in memory as
  long as they have not changed.
- Folders on Google Drive, OneDrive and Dropbox open faster: Nuage asks
  the provider for their content less often, and still sees changes made
  elsewhere within a minute.
- Nuage wakes up less often when nothing is syncing, which saves battery.
- Security: Nuage now uses SQLite 3.54, which fixes a flaw in reading
  damaged database files.

## [0.2.2] - 2026-11-07

- Nuage says when a new version is available, with an **Update** button,
  then offers to restart once it is installed. No need to wait for
  Software or to use a terminal.

## [0.2.1] - 2026-11-05

- A homepage: <https://felixilgatto.github.io/nuage/>.
- Each account's settings now open from a gear button on its row in the
  main window. Preferences keeps only the general settings, so adding many
  accounts no longer crowds its top bar.

## [0.2.0] - 2026-11-05

- Install on the desktop from Nuage's own Flatpak repository
  (`flatpak install --user https://felixilgatto.github.io/nuage/nuage.flatpakref`):
  Nuage then updates with `flatpak update` and in Software. The `.flatpak`
  bundles of the releases update from it too.
- Packages for ARM64 (Raspberry Pi, ARM servers) besides x86-64, and an
  `.rpm` for Fedora and openSUSE besides the `.deb` and the tarball.
- Each download comes with a checksum and a build provenance:
  `gh attestation verify <file> --repo felixilgatto/nuage` proves it was
  built by the release workflow.
- New providers: Nextcloud and WebDAV (server address and password), and S3
  bucket storage (Amazon S3, Cloudflare R2, Backblaze B2, MinIO…, with an
  access key; one bucket, or all of them).
- In preview, not yet tried with real accounts: Dropbox and pCloud (sign in
  in the browser), and Proton Drive as a build option.
- Nuage checks the connection before adding an account set up with a form
  (Nextcloud, WebDAV, S3, Proton Drive): a wrong password or bucket is
  reported at once, with what to check, instead of giving a folder that
  shows nothing.
- Folders mount after a restart even when the keyring is slow to answer at
  login (a session that logs in automatically, or a desktop portal still
  starting): Nuage waits for it instead of stopping. Meanwhile, the tray shows
  "Waiting for the keyring" with a key, not a problem; it becomes a problem
  after 3 minutes. The window starts the Nuage service again if it crashes.
- Removing an account no longer fills the log with errors about its cache.
- After an update installed while Nuage runs, folders still mount, and Nuage
  offers to restart into the new version.
- Leaving the folder screen without starting the sync no longer leaves a
  half-added account; going back is locked while the folder is set up.
- After adding an account, the window stays open on the accounts list, with
  "Open Folder"; "Start Sync" shows that it is working.
- How to uninstall, in the README.
- OneDrive (personal): sign in in the browser, no client to create. Nuage
  picks your own drive (the one named "OneDrive"), and the next one if
  Microsoft refuses it; on a server, `--token-file` works the same way. The
  Personal Vault, which cannot be opened, is documented.
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
