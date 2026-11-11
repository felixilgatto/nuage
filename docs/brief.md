# Brief — Nuage

**Cloud storage client for Linux: a Rust daemon that drives rclone, a GNOME app (Flatpak) for the desktop and a CLI for headless servers.**

## In short

On Linux, mounting a Google Drive or a OneDrive is painful: rclone can do everything but has no simple interface. Nuage:

- mounts each cloud account in a folder chosen by the user, visible in Nautilus;
- on the desktop, shows the sync status and the space used in a system tray icon;
- offers **a single file browser, identical for all providers**;
- is set up in **a few guided screens** (4 for a first Google account, 3 after that), then lives in the system tray;
- **is installed and used entirely from the command line** on a headless server reached over SSH, including by an agent such as Claude Code.

v1: Google Drive, **public from v1**. The architecture is multi-provider from the start.

**Status as of 2026-11-05**: v1, v1.1 and v2 (milestones 1 to 14) are done; 0.2.0 was released on 2026-11-05 (see [Milestones](#milestones)). Available providers: Google Drive (with shared drives), OneDrive, Nextcloud, WebDAV, S3 storage (tried with real accounts or servers). In *preview* (not yet tried with a real account): Dropbox, pCloud, and Proton Drive (feature outside `default`). `local` for tests.

## Instructions for Claude Code

1. **Order of work**: follow the [milestones](#milestones) in order. Milestone 1 (proof that FUSE works from the Flatpak) comes before everything else; if it fails, stop and discuss it.
2. **Git**: the repository is `git@github.com:felixilgatto/nuage.git` (https://github.com/felixilgatto/nuage), created **manually** by Felix. Do not create any other remote repository. Commit and push only on request.
3. **Language**: **everything in the repository is in English**, including this brief: code, identifiers (including enum variants), comments, commit messages, README, `docs/`, UI and CLI strings. UI strings go through gettext so they can be translated later.
4. **Modularity**: the modular architecture (see [Architecture](#architecture)) is in place from the first commit, even with a single provider.
5. **MIT license**: `LICENSE` file at the root, `license = "MIT"` in every `Cargo.toml`, `<project_license>MIT</project_license>` in the `metainfo.xml`. `cargo-deny` only accepts MIT-compatible licenses (MIT, Apache-2.0, BSD, ISC, Zlib; MPL-2.0 case by case; GPL/AGPL refused). rclone is under MIT, so it can be bundled.
6. **Names**: application **Nuage**; binaries `nuaged` (daemon), `nuage` (CLI), `nuage-gtk` (desktop window), `nuage-tray` (desktop agent: tray icon, notifications); crates `nuage-*`; Flatpak ID **`io.github.felixilgatto.Nuage`**.
7. **Points to verify**: unconfirmed technical assumptions are grouped in [To verify](#to-verify). Verify them at the indicated time rather than assuming they are true.

## Principles

These rules settle cases the brief does not cover.

- **We do not rewrite a sync engine.** rclone does the mounting, the cache, OAuth and the transfers. Nuage is an interface on top of its remote control API (rc). Only exception: the overlay file system `nuage-fs` (exact exclusions in v1, offline pinning in v1.1).
- **One engine, several interfaces.** The engine runs in a daemon (`nuaged`). The GTK app, the system tray icon and the CLI are its clients. **Everything the graphical interface does can be done from the CLI**, except the graphical browser.
- **First-class headless.** On a server without a screen, Nuage is installed, configured and used only from the command line, with no mandatory interactive step.
- **Public from v1.** Error messages understandable by a non-technical person, with the action to take; step-by-step installation README.
- **Files only.** No calendar, contacts, mail or photos. OAuth scopes are limited to files.
- **Minimalist interface.** As few visible screens, choices and settings as possible. Anything with a good default is not asked and stays in the settings. No welcome screen, tutorial or summary.
- **Provider independence.** Everything that depends on a provider is in its module. Interfaces adapt to the provider's **capabilities**, never to its name.
- **Native GNOME** on the desktop: GTK4 + libadwaita, GNOME HIG, Flathub quality guidelines.
- **Sandbox as closed as possible.** Flatpak permissions are minimal and each one is justified in `docs/INSTALL.md`.

## Scope by version

| Feature | v1 | v1.1 | v2 | Never |
|---|---|---|---|---|
| Google Drive ("My Drive") | ✔ | | | |
| Shared drives (each one an account) | | | ✔ | |
| "Shared with me" (Google), SharePoint and OneDrive for Business | | | later | |
| `local` provider (tests, CI) | ✔ | | | |
| OneDrive (personal) | | | ✔ | |
| Nextcloud, WebDAV, S3 storage | | | ✔ (added on 2026-11-05) | |
| Dropbox, pCloud | | | preview (to try with a real account) | |
| Proton Drive | | | preview, opt-in build (rclone backend in beta) | |
| Mount per account, folder of your choice | ✔ | | | |
| `nuaged` daemon + `nuage` CLI | ✔ | | | |
| Headless installation (native binary, systemd service) | ✔ | | | |
| GTK app + system tray icon (Flatpak) | ✔ | | | |
| Quota per account | ✔ | | | |
| Local cache limit, per account | ✔ | | | |
| Pause / resume uploads | ✔ | | | |
| Unified graphical file browser | ✔ | | | |
| File operations from the CLI (`ls`, `cp`, `mv`…) | ✔ | | | |
| Guided setup (including the user's own OAuth client), autostart | ✔ | | | |
| Overlay file system `nuage-fs` | ✔ relay + exclusions | ✔ + pinning | | |
| Exclusions (paths and patterns) | ✔ via `nuage-fs` + rclone filters | | | |
| Offline mode (pinning) | | ✔ | | |
| GitHub Actions CI, releases (Flatpak bundle + native binaries) | ✔ | | | |
| Publication on Flathub | | | ✔ | |
| Text (TUI) or web interface | | | | ✘ (the CLI is enough) |
| Status badges in Nautilus | | | outside Flatpak, possibly | impossible from a Flatpak |
| Offline mirror of the whole account | | | | ✘ (only pinned items) |
| Conflict resolution for unpinned files | | | | ✘ (we rely on rclone's VFS) |
| Editing Google Docs outside the web browser | | | | ✘ (web shortcuts, see [Google Drive](#accounts-and-authentication)) |
| Calendar, contacts, mail, photos, collaborative documents | | | | ✘ |

## Existing tools (assessed on 2026-10-30)

**RClone Manager** (`io.github.zarestia_dev.rclone-manager`, Flathub, Tauri/Rust, maintained) covers part of the need on the desktop: mounting, tray icon, several providers, file browser. But:

- **its Flatpak does not mount as is**: the manifest declares neither `--talk-name=org.freedesktop.Flatpak` nor `--filesystem=home`, and the user has to run a manual `flatpak override` (issues #45, #52, #113; the maintainer says Flathub refuses these permissions);
- **rclone is not bundled**: the app downloads the latest version on first launch, with no pinned version;
- it shows the quota (`operations/about`) but **no "up to date / syncing" status** (it does not read `vfs/stats`);
- it is not native GNOME (Tauri/WebKit).

Nuage is therefore justified by a native, simple and focused GNOME UX, mounting that works without manual tweaking, a clear sync status, and a scriptable headless mode. Its `fusermount3` wrapper remains a useful reference.

## Two distribution modes

| | Desktop | Headless |
|---|---|---|
| Target | Ubuntu / GNOME with a graphical session | Linux server without a screen, administered over SSH |
| Package | Flatpak (`.flatpak` bundle, Flathub in v2) | native binary: x86_64 tarball and `.deb` |
| Contents | `nuaged` + `nuage-gtk` + `nuage-tray` + `nuage` + rclone + `fusermount3` wrapper | `nuaged` + `nuage` + rclone (same pinned version) + systemd unit; **no GTK dependency** |
| Daemon launch | by `nuage-gtk` or `nuage-tray` (autostart of `nuage-tray` via the Background portal) | systemd user service `nuaged.service`; `loginctl enable-linger` to run without an open session |
| FUSE | the host's `fusermount3`, via the wrapper (see [hard part](#the-hard-part-fuse-in-the-flatpak-sandbox)) | the system's `fusermount3`, directly |
| Secrets | Secret portal | Secret Service if available, otherwise a `0600` file (see [Security](#security-and-storage)) |
| Authentication | browser opened automatically | URL + SSH tunnel, or token generated elsewhere (see [Authentication](#accounts-and-authentication)) |
| Host prerequisites | `flatpak`, `fuse3` | `fuse3`, systemd |

## The hard part: FUSE in the Flatpak sandbox

Only concerns the Flatpak distribution. The sandbox has neither `fusermount3` (setuid binary) nor the right to mount. The **proven** workaround is GNOME Builder's, reused by RClone Manager:

1. The Flatpak installs a script `/app/bin/fusermount3` that replaces the real one.
2. When rclone (in the sandbox) mounts, it calls this script, which relays the call to the **host's `fusermount3`** via `flatpak-spawn --host`, passing the file descriptors (`_FUSE_COMMFD`, `/dev/fd/N`) with `--forward-fd`.
3. The mount is done by the host, so it is **visible in Nautilus**. rclone stays in the sandbox and reads the `/dev/fuse` descriptor sent back to it.

**Complication on Ubuntu 26.04 (found at milestone 1).** The AppArmor profile of `fusermount3` (`/etc/apparmor.d/fusermount3`, fuse3 3.18) allows no inherited Unix socket. A socket created in the sandbox (label `unpriv_bwrap`) is denied (`apparmor="DENIED" operation="file_inherit" ... peer="unpriv_bwrap"`), and `fusermount3` answers "file descriptor 3 is not a socket". The GNOME Builder and RClone Manager method therefore fails as is on this version.

**Chosen workaround**: a **host-side relay**. The wrapper launches, via `flatpak-spawn --host`, a small program that creates a socket pair on the host itself, runs `fusermount3` with it, receives the `/dev/fuse` descriptor and sends it back to rclone through the socket passed from the sandbox. No change to the system is needed.

- Prototype (`poc/flatpak-fuse/`): relay in Python run by the host's `python3`. Mounting, reading, writing from the host and unmounting validated on 2026-10-30.
- Final version: a small Rust binary `nuage-fuse-relay`, shipped in the Flatpak and run on the host from the app's path (read from `/.flatpak-info`), so as not to depend on Python on the host.
- To report to Ubuntu (Launchpad, `fuse3` package): the profile also breaks GNOME Builder.

Reference: `fusermount-wrapper.sh` in `github.com/flathub/org.gnome.Builder` and `github.com/flathub/io.github.zarestia_dev.rclone-manager`.

**Consequences, to document in the README:**

- **The sandbox is pierced**: the `--talk-name=org.freedesktop.Flatpak` permission allows running commands on the host. Acceptable for a v1 distributed as a bundle; to be justified for Flathub (Builder obtained it).
- **Limited use**: the wrapper relays **only** `fusermount3`. No other call to `flatpak-spawn --host` in the app.
- **Host dependency**: `fuse3` (installed by default on Ubuntu).
- **Same path on both sides**: it is the host's `fusermount3` that mounts, with the path rclone passes to it; the mount folder must therefore exist at the same path in the sandbox and on the host. Since the user chooses their folder freely, we request `--filesystem=home`.

## Architecture

### Modularity

Everything that depends on a provider is isolated behind a `Provider` trait. Since rclone does the I/O for all backends, **a provider is essentially declarative**: rclone backend type, authentication flow, parameters, capabilities, recommended options. The engine, the interfaces and the CI only know the abstraction.

> **Adding a provider = adding a module, without touching `engine`, `ui`, `tray` or `cli`.**

### Processes

```
nuaged  (daemon: engine + rclone rcd)
   ▲  Unix socket, JSON (Command / Event)
   ├─ nuage-tray  desktop agent: system tray icon, notifications, updates
   ├─ nuage-gtk   desktop app: the window
   └─ nuage       CLI (humans, scripts, agents)
```

- The daemon owns all the state: accounts, mounts, exclusions, pins. Clients never talk to rclone.
- Closing or quitting the GTK app does not necessarily stop the sync (see "Quit" in [Interface](#interface-gnome--libadwaita)).
- A single daemon per user; a second launch detects the first through the socket and exits.

### Cargo workspace

```
nuage/
├─ Cargo.toml                      (workspace)
├─ crates/
│  ├─ core/        nuage-core       domain model, traits, Command/Event (no GTK dependency)
│  ├─ ipc/         nuage-ipc        Unix socket protocol: serialization, client, server
│  ├─ rclone-rc/   nuage-rclone     typed client for the rc API + supervision of the rcd process
│  ├─ providers/   nuage-providers  Provider trait + registry + one module per provider
│  │   └─ src/{gdrive, onedrive, dropbox, pcloud, protondrive, nextcloud, webdav, s3, local}.rs   (one Cargo feature per provider)
│  ├─ engine/      nuage-engine     accounts, mounts, exclusions, pinning, status aggregation
│  ├─ fs/          nuage-fs         overlay FUSE file system (relay + exclusions in v1, pinning in v1.1)
│  ├─ daemon/      nuaged           binary: engine + ipc server
│  ├─ cli/         nuage            binary: CLI (clap), ipc client
│  ├─ tray/        nuage-tray       desktop agent: StatusNotifierItem icon (ksni), notifications
│  ├─ ui/          nuage-ui         GTK4 + libadwaita: browser, setup, settings
│  └─ gtk/         nuage-gtk        binary: the window, assembles ui, ipc client
├─ build-aux/flatpak/              manifest, fusermount-wrapper.sh, cargo-sources.json
├─ build-aux/native/               systemd unit, cargo-deb configuration, tarball script
├─ data/                           .desktop, metainfo.xml, icons, schemas
├─ docs/ARCHITECTURE.md
├─ docs/ADDING_A_PROVIDER.md
├─ docs/CLI.md
├─ .github/workflows/              CI
└─ README.md
```

**Dependency rules** (checked in CI):

| Crate | May depend on |
|---|---|
| `core` | no crate of the project |
| `ipc` | `core` |
| `providers` | `core` |
| `rclone-rc` | `core` |
| `engine` | `core`, `rclone-rc`, `providers`, `fs` |
| `daemon` | `engine`, `ipc`, `core` |
| `cli`, `ui`, `tray`, `gtk` | `core`, `ipc` (+ `tray` for `ui`, `ui` and `tray` for `gtk`); **never** `engine` or `rclone-rc` |

`daemon` and `cli` must pull in **no GTK dependency** (checked in CI with `cargo tree`), so that the headless package installs on a bare server.

### IPC protocol (`ipc`)

- Unix socket `$XDG_RUNTIME_DIR/nuage/nuaged.sock`, permissions `0600`. In the Flatpak: `$XDG_RUNTIME_DIR/app/io.github.felixilgatto.Nuage/nuaged.sock`, shared with the host, so that the host's CLI can connect to it too.
- Newline-delimited JSON: `Command` requests with a response, plus a subscription to the `Event` stream.
- Handshake with a protocol version number; a client that is too old or too new gets a clear error.

### Domain model (`core`)

```rust
AccountId
Account   { id, provider_id, display_name, remote_name, root, mount_point, cache_limit }
          // root: folder of the remote shown by the account (an S3 bucket…), empty = the whole remote
RemotePath { account, path }
Entry     { name, path, kind: EntryKind /* File | Folder */, size, modified, mime, hash,
            native_doc: Option<NativeDoc> }      // Google Docs: web link
Quota     { total, used, free, trashed }         // all optional: not every backend provides them
SyncState  = UpToDate | Syncing { uploads, queued } | Paused { queued } | Error { … } | Stopped | Offline
Capabilities                                      // see "Provider trait"
Exclusion { account, rule: ExclusionRule /* Path(RemotePath) | Glob(String) */, created_at }
Pin       { account, path, kind: PinKind /* File | FolderRecursive */, pinned_at }   // v1.1
PinState   = Downloading | Available | ModifiedLocally | Conflict | Error             // v1.1
Command   // client → daemon
Event     // daemon → clients
```

`Account.mount_point` is the **folder chosen by the user**; it must not be confused with rclone's internal mount point (see [Offline mode](#offline-mode-v11)).

### `Provider` trait

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;                    // "gdrive"
    fn display_name(&self) -> &str;                  // "Google Drive"
    fn icon_name(&self) -> &str;
    fn rclone_backend(&self) -> &'static str;        // "drive"
    fn auth(&self) -> AuthSpec;                      // OAuthBrowser | Credentials(form) | None
    fn config_params(&self, input: &SetupInput) -> Vec<(String, String)>; // e.g. ("scope", "drive")
    fn static_capabilities(&self) -> Capabilities;   // completed at runtime by operations/fsinfo
    fn recommended_vfs_opts(&self) -> VfsOptions;
    fn native_docs(&self) -> Option<NativeDocPolicy>; // Google Docs → web shortcuts
    // Optional (default implementation):
    fn client_guide(&self) -> Vec<GuideStep>;        // steps to create your own OAuth client (gdrive)
    fn client_id_suffix(&self) -> Option<&'static str>;
    fn sub_drives(&self) -> Option<SubDrives>;       // shared drives: one account each
    fn choice_rank(&self, question: &str, choice: &str) -> u8; // rclone's questions after OAuth (OneDrive drive)
    fn root(&self, input: &SetupInput) -> String;    // folder of the remote shown (S3 bucket)
    fn validate(&self, input: &SetupInput) -> Result<(), String>;
}
```

Up-to-date detail, with one line per method: `docs/ADDING_A_PROVIDER.md`.

- **Registry**: `ProviderRegistry`, built from the enabled Cargo features.
- **Actual capabilities** = static capabilities merged with `operations/fsinfo` (rclone exposes the backend's `Features` there: `About`, `PublicLink`, `Copy`, `Move`, `PurgeTrash`…). **Interfaces hide or refuse actions based on `Capabilities`.**
- **`local` provider** (an rclone `alias` of a folder): for tests and CI, without credentials.
- **Shipped** (`default` features): `gdrive`, `onedrive`, `nextcloud`, `webdav`, `s3`, `local`; in **preview**: `dropbox`, `pcloud`. **Preview outside `default`**: `protondrive`. The preview status is marked by a `# Preview` comment in `crates/providers/Cargo.toml`; preview providers come last in the list.

### Engine (`engine`, in `nuaged`)

```
nuaged
 ├─ RcdSupervisor     launches and monitors `rclone rcd` (127.0.0.1, random port and credentials)
 ├─ AccountManager    creating, renaming, removing, reconnecting accounts
 ├─ MountManager      mount/mount, mount/unmount, remounting at startup, cleaning up orphans
 ├─ UploadControl     pause / resume uploads, per account or global
 ├─ ExclusionManager  exclusion rules per account → rclone filters of the mount
 ├─ StatusPoller      polls vfs/stats, core/stats, operations/about; publishes Events
 ├─ FileService       file operations (list, mkdir, move, copy, delete, upload, download, link)
 └─ PinManager        offline pinning (v1.1)
```

In the Flatpak, `/app/bin` contains `rclone` (pinned version), the `fusermount3` wrapper, `nuaged`, `nuage`, `nuage-gtk` and `nuage-tray`.

**Lifecycle:**

- rclone is a child process of the daemon.
- At startup: clean up orphan mounts left by a crash, then remount the accounts.
- At shutdown (`SIGTERM`, `nuage daemon stop`, "Quit"): unmount all accounts, then `core/quit`. The behavior when uploads remain is described in [Quit](#quit).

### rc endpoints used

| Need | Endpoint | Note |
|---|---|---|
| Daemon alive | `core/version` | health check, minimum version |
| Capabilities | `operations/fsinfo` | backend features |
| Mount | `mount/mount` | `fs`, `mountPoint`, `vfsOpt` (`CacheMode=full`, `CacheMaxSize`, `CacheMaxAge`, very long `WriteBack`), `_filter` (`ExcludeRule` / `FilterRule`); all in JSON |
| List / unmount | `mount/listmounts`, `mount/unmount` | |
| Sync status | `vfs/stats` | `fs` **required** as soon as several VFS are active; `diskCache.{uploadsInProgress, uploadsQueued, erroredFiles, bytesUsed, outOfSpace}`; `bytesUsed` is only refreshed at the cache poll (60 s by default) |
| Upload queue | `vfs/queue` | `{"queue":[{name, id, size, expiry, tries, delay, uploading}]}`; "uploading" badge per file |
| Pausing uploads | `vfs/queue` + `vfs/queue-set-expiry` | see [Pause](#sync-status-pause-and-quota) |
| Quota | `operations/about` | `total`, `used`, `free`, `trashed`; missing fields are omitted; check `Features.About` before the call (the `memory` backend returns a 500 error) |
| Refresh | `vfs/refresh` | |
| Browsing | `operations/list`, `operations/stat` | |
| File operations | `operations/mkdir`, `movefile`, `copyfile`, `deletefile`, `purge`, `uploadfile`, `publiclink` | `_async=true` + `job/status` for long operations |
| Transfers | `core/stats` | speed, progress |
| Pinning (v1.1) | `operations/copyfile`, `operations/stat`, `operations/list` (hashes, modtime) | download, upload back, detection of remote changes |
| Accounts | `config/create`, `config/update`, `config/listremotes`, `config/get`, `config/delete` | non-interactive state machine (`opt: {nonInteractive, obscure, continue, state, result}`); `config/create` saves the remote before the questions are over: delete it if the user gives up |
| OAuth (GUI) | `config/create` with `_async`, `config/oauthstatus`, `config/oauthstop` | `config_auth_no_browser: "true"` then `oauthstatus` gives the `authUrl` to open via OpenURI; `oauthstop` cancels |
| Shutdown | `core/quit` | |

Field and option names change from one rclone version to another: check them on the bundled version. An integration test must fail if an expected field disappears during a version upgrade.

### What "up to date" means

In mount mode, nothing is copied in advance. **"Up to date" = no local write waiting to be uploaded.** Changes made elsewhere appear at the next refresh (backend polling by rclone) or via "Refresh". Providers that report changes (Drive, OneDrive, Dropbox) poll every minute and keep listings for an hour (`DirCacheTime`), so folders open without a request; the hour bounds a missed change.

## Features

### Accounts and authentication

- Setup (GUI) or `nuage account add` (CLI) lists the providers in the registry, then follows the `AuthSpec` of the chosen provider.
- **Default account name**: the provider's name ("Google Drive"), with a suffix if it is already taken ("Google Drive 2"). It is also used for the default folder. Can be renamed later; the email address is shown as a subtitle when the provider gives it.
- Expired or revoked token → ⚠ state and "Reconnect" action (`rclone config reconnect`, or `nuage account reconnect`).

**Removing an account** (after confirmation; in the CLI, `--yes` is required): unmounting, removal of the rclone remote, of the cache, of the pins and of the mount folder if it is empty. **Nothing is deleted in the cloud.** Refused while uploads are still pending, unless `--force` is given.

**OAuth on the desktop**: `rclone config create <name> <backend> …`; rclone opens its local server `127.0.0.1:53682` and the browser (via the OpenURI portal from the sandbox). Error cases to handle: port in use, cancellation, timeout, browser that does not open (show the URL to copy).

**Headless OAuth** (two methods, documented in the README):

1. **URL + SSH tunnel** (default without a local browser): the CLI shows the authorization URL and waits on `127.0.0.1:53682`. The user opens a tunnel (`ssh -L 53682:localhost:53682 server`) and opens the URL on their own machine. On the rclone side: parameter `config_auth_no_browser: "true"` (the `--auth-no-open-browser` flag only exists for `rclone authorize`).
2. **Token generated elsewhere** (fully scriptable): `rclone authorize drive <client_id> <client_secret>` on a machine with a browser, then `nuage account add gdrive --client-id … --client-secret … --token-file token.json` (or `--token '<json>'`) on the server. On the rclone side, a single `config/create` is enough by providing `token`, `config_refresh_token: "false"`, `config_change_team_drive: "false"`.

**Google Drive:**

- "My Drive", and each shared drive as a separate account, without signing in again (`sub_drives`). "Shared with me" later.
- `drive` scope: access to files only, no other Google service;
- **The user's own OAuth client is required, with a guided flow.** rclone's shared client "will stop working during 2026" (rclone 1.75 warning), and a client of Nuage's own would require a Google verification + a CASA audit (`drive` scope is "restricted"). Each user therefore creates their own GCP client (type "Desktop app"):
  - GUI: a guided screen before authentication (numbered steps, direct links to the GCP console, client ID / secret fields to paste into, immediate check);
  - CLI: `--client-id` / `--client-secret` required for `gdrive`, with a message pointing to the README guide if they are missing;
  - the client is remembered globally: subsequent Google accounts do not go through this screen again;
- `drive` is a "restricted" scope: in *Testing* status, refresh tokens expire after 7 days. The guide has the user publish the GCP project in *Production* (unverified, sufficient for personal use);
- **Native Google Docs/Sheets/Slides**: presented as **web shortcuts** in the mounted folder, in rclone's **`link.html`** export format (a page that redirects to the document). The `desktop` format opens in the text editor from Nautilus, so it is ruled out. A double-click opens `link.html` with the default HTML application (normally the browser). Link files are read-only (rclone refuses to modify them). No docx/xlsx/pptx export, so neither duplicates nor loss of formatting. In Nuage's browser, they are flagged and open directly in the web browser.

**OneDrive**: the account's personal OneDrive; scopes `Files.ReadWrite.All` + `offline_access` only (rclone default); rclone's client by default, own Azure app optional. The drive named "OneDrive" is tried first (`choice_rank`). The Personal Vault does not open outside Microsoft apps.

**Dropbox, pCloud**: OAuth in the browser with rclone's client (own client optional), like OneDrive. pCloud finds the region (US or EU) at authentication.

**Credential-based providers** (`AuthSpec::Credentials`, a form; in the CLI `--param key=value`):

- **Nextcloud**: server address (Nuage derives `…/remote.php/dav/files/<user>/` from it) or WebDAV address copied from Nextcloud, user name, app password; rclone vendor `nextcloud` (chunked uploads).
- **WebDAV**: any WebDAV server; full address, optional user and password.
- **S3**: Amazon S3 and compatibles (R2, B2, Wasabi, MinIO…); access keys, endpoint (empty for Amazon), region if needed, and **optional bucket**: if given, the account shows this bucket (or `bucket/folder`) via `Account.root`, which works with a key limited to that bucket; otherwise each bucket is a folder.
- **Proton Drive** (preview, opt-in build): address, password, 2FA code; rclone keeps the session. A session ended by Proton does not reconnect: remove and add the account again. A separate mailbox password is not supported.
- rclone saves a form without trying it: **the engine lists the account's root right after creating it** (30 s at most) and, on failure, deletes the remote and says what to fix (filled-in fields to check, bucket that does not exist, unreachable server).
- No pushed changes from these backends: a `DirCacheTime` of one minute to see changes made elsewhere.

### Mounting and mount folder

- One mount folder **per account**, chosen by the user (`gtk::FileDialog` or `--mount-point`). Default: `~/Cloud/<account-name>`.
- Validations: folder empty or nonexistent (we create it), not already used by another account, not nested in another mount point, not in the cache.
- **Flatpak, under the home**: covered by `--filesystem=home`. The FileChooser portal then returns the real path. Reject any path under `/run/user/*/doc/` with a clear message.
- **Flatpak, outside the home** (e.g. `/mnt/data/Drive`): the app detects it and shows the command to run: `flatpak override --user --filesystem=/mnt/data/Drive io.github.felixilgatto.Nuage`.
- **Native**: no restriction beyond the user's permissions.
- Changing the folder unmounts the old one and remounts at the new one.

### Local cache

- Files downloaded on demand, into rclone's VFS cache (`CacheMode=full`).
- **Limit per account**, **20 GB by default**. Beyond that, the oldest files are evicted. The interface also shows the total for all accounts.
- Maximum cache age: reasonable default, can be changed in the settings.

### Sync status, pause and quota

**Aggregated status** over all accounts, shown by the system tray icon and by `nuage status`:

| Icon | Status | Condition |
|---|---|---|
| ✓ | up to date | no local write waiting to be uploaded |
| ↻ | syncing | n files uploading or queued |
| ⚠ | problem | error, disconnected account or failed files |
| ⏸ | paused | uploads suspended or mount stopped |

**Pause**: suspends **uploads**, per account or for all. The folder stays mounted and usable; writes pile up in the cache and are sent on resume. The pause survives a daemon restart.

Mechanism (verified on rclone 1.75.2): each account is mounted with a very long `WriteBack` (e.g. `8760h`), so rclone sends nothing on its own. When the account is not paused, `UploadControl` polls `vfs/queue` every second while uploads may be going on (after a file is written through the user's folder, a mount or a resume, and while the queue is not empty), every 20 seconds otherwise, and releases each new item with `vfs/queue-set-expiry` (`expiry: -1e9`). When paused, it releases nothing. The pause state is in Nuage's TOML config. Pending writes survive an rclone shutdown and are queued again on remount. `core/bwlimit` is ruled out: it is global and slows down without ever suspending.

**Quota** per account: used / total / free / trash, with a percentage. Fields missing if the backend does not provide them.

### Unified file browser (GUI)

**A single design for all providers.** The components are generic and only know `Entry`, `Capabilities` and the file `Command`s. Browsing goes through the rc API, **independently of the mount**: it works even when the account is unmounted.

**Structure** (`AdwNavigationSplitView`):

- **Sidebar**: the accounts, each with the provider's icon and a quota bar. In v1.1, an "Available offline" entry lists all pinned items.
- **Content**: breadcrumbs, list view (`GtkColumnView`: name, size, date, status) or grid view (`GtkGridView`); the choice is remembered.
- **Header bar**: back, path, filter for the current folder, new folder, upload files, menu.

**Actions**, shown only if the capability exists:

- open, download to…, upload (button or drag and drop from Nautilus);
- rename, move, copy, new folder;
- delete (to the trash if supported, otherwise with confirmation);
- copy share link, properties, "Open in Files" (mounted folder);
- "Exclude from sync" / "Include in sync" (see [Exclusions](#sync-exclusions));
- v1.1: "Keep offline" / "Remove offline copy", with a padlock icon.

**Opening a file**: if the account is mounted, open its path in the mount via the OpenURI portal (default host app); otherwise, download it to a temporary folder then open it. A native Google Doc opens in the web browser.

**Per-file badges**:

| Badge | Version | Source |
|---|---|---|
| Uploading | v1 | `vfs/queue` |
| Not synced (excluded item, greyed out) | v1 | exclusion rules |
| Online only (offline, not pinned, greyed out) | v1 | cached listings |
| Available offline / Modified locally / Conflict / Error | v1.1 | `PinState` |

**Behavior**: large folders loaded with pagination or progressively; no blocking call on the GTK thread; errors shown in an `AdwToast`.

### Sync exclusions

**Need.** Per account, the user chooses what they **do not want to sync**:

- specific remote files or folders (e.g. a large "Archives" folder);
- patterns that apply to the whole account (e.g. `*.tmp`, `~$*`, `.~lock.*`, `node_modules/`, `.git/`).

An excluded item is **neither downloaded, nor cached, nor uploaded**, and does not appear in the mounted folder. It stays intact in the cloud.

**Rules:**

- Patterns suggested by default (can be turned off): temporary files and Office/LibreOffice lock files (`~$*`, `.~lock.*#`, `*.tmp`, `.goutputstream-*`).
- **Pinning and excluding are mutually exclusive.** Excluding a pinned item asks for confirmation, then unpins it (after uploading its changes). An excluded item cannot be pinned (action hidden in the GUI, error in the CLI).
- Persistence: the app's TOML config, per account (not in `rclone.conf`).

**Implementation:**

rclone filters alone are not enough (verified on rclone 1.75.2): `_filter` on `mount/mount` does hide the excluded remote items and prevents reading them, but an excluded file **created locally** in the mount is accepted **and uploaded**. Hence two layers, from v1:

- **Remote side: rclone filters** on the hidden rclone mount (`_filter` → `ExcludeRule` / `FilterRule`). Excluded remote items are neither listed nor downloaded. Changing an exclusion **remounts the hidden rclone mount** (not the user's folder, which `nuage-fs` keeps in place), once the upload queue is empty; otherwise we ask for confirmation (`--yes` in the CLI).
- **Local side: `nuage-fs`** (see [Overlay file system](#overlay-file-system-nuage-fs)). A file created or renamed locally that matches an exclusion is never passed to rclone: it is **kept only locally** (`local-only` folder, see [Security and storage](#security-and-storage)) and stays visible at its path in the mounted folder.

**Interface:**

- Browser: "Exclude from sync" / "Include in sync" in the context menu. Excluded items stay visible in the browser (which does not go through the filters), greyed out with the "Not synced" badge, so they can be included again.
- Account settings: "Excluded from sync" list (paths and patterns), adding a pattern, removing a rule.
- CLI: `nuage exclude add|remove|list`.
- Nothing in the setup: by default, everything is synced.

### Overlay file system (`nuage-fs`)

From v1, the user does **not** see the rclone mount directly. The daemon mounts its own FUSE file system (`fuser` crate) at the user's mount point, on top of a hidden rclone mount:

```
~/Cloud/Google Drive                  ← folder seen by the user (nuage-fs)
   ├─ excluded item, created locally  → local-only folder (v1)
   ├─ pinned item                      → local pin store (v1.1)
   └─ everything else                  → relayed to the hidden rclone mount
                                          data/mounts/<account>  (limited VFS cache, exclusion filters)
```

**v1: relay + exclusions.**

- Transparent relay of all operations to the hidden rclone mount (read, write, rename, attributes, listing), with no noticeable regression in performance or behavior compared with a direct rclone mount.
- Interception of creations and renames that match an exclusion: these files go to `local-only` and are never seen by rclone. Their listing is merged with rclone's.
- The user mount stays in place when the hidden rclone mount is remounted (exclusion change, reconnection): ongoing accesses wait or fail cleanly, without the folder disappearing.
- In the Flatpak, `nuage-fs` goes through the same `fusermount3` wrapper as rclone: no extra permission.

**v1.1: offline pinning**, described below.

This is the most delicate building block of the project. It must be covered by dedicated integration tests (relay, exclusions, remount) with the `local` provider, and pass a file system test suite (e.g. `pjdfstest` or equivalent, at minimum the read/write/rename/delete cases).

### Offline mode (v1.1)

**Need.** The user **pins** a file or a folder. The item stays **at the same path in the mounted folder** and can be opened and modified **without network**, including **after an offline restart**. This is the equivalent of the "Available offline" of OneDrive or Google Drive for desktop. Pinned items are **outside the cache**: never evicted, whatever the limit.

**Why rclone's VFS cache is not enough:**

- it has **no pinning**: a file can be evicted as soon as the cache reaches its limit;
- the directory cache is **in memory**: after a restart without network, rclone can no longer list folders and cached files become inaccessible by their path;
- the rclone mount may **refuse to start** if the backend is unreachable (token refresh);
- offline changes are uploaded on a "last writer wins" basis, without conflict detection.

**Chosen design: pinning is added to `nuage-fs`** (see [Overlay file system](#overlay-file-system-nuage-fs)). Pinned items are read and written in a local store; the rest is still relayed to rclone.

| Situation | Behavior |
|---|---|
| Reading a pinned file | from the pin store |
| Reading an unpinned file | relayed to rclone; offline → clean error (`EHOSTUNREACH`), the item stays listed thanks to the metadata |
| Writing to a pinned file | written locally, marked "modified", uploaded by `operations/copyfile` as soon as the network is back (unless paused) |
| Writing to an unpinned file | relayed to rclone (its write-back cache handles the upload) |
| Remote change to a pinned item | detected by periodic check (`operations/list` with hash and date, adjustable interval), new version downloaded |
| Change on both sides since the reference state | **conflict**: both versions are kept (`report (conflict 2026-10-29 18-45).docx`), never any loss, and a notification |
| New file in a pinned folder (local or remote) | pinned automatically (pinning a folder is recursive) |
| Startup without network | `nuage-fs` mounts from the metadata even if rclone fails, then switches to rclone as soon as it is available |
| Unpinning | the local copy is released once the local changes are uploaded |

**Storage:**

- **Pin store**: full copies, outside the VFS cache limit.
- **Metadata** (SQLite, `rusqlite`): list of pins; for each pinned file, its reference state (size, date, remote hash, local hash); **folder listings** (at minimum those of the pinned items and their parents) so that the tree stays browsable offline and after a restart.
- **Space**: the size of the store is shown separately from the cache ("Offline: 4.2 GB"), with an optional warning beyond a threshold chosen by the user.

This is the heaviest milestone of the project: part of Nuage becomes a real sync client (for pinned items only). Transfers are still delegated to rclone.

**Interface:**

- Browser: "Keep offline" / "Remove offline copy" (context menu and padlock button), status badges, "Available offline" entry in the sidebar.
- Account settings: "Available offline" list of pinned items, with their size and the "Remove offline copy" action.
- Tray: number of pinned items waiting to be uploaded or in conflict.
- Settings: check interval, space warning threshold.
- CLI: `nuage pin add|remove|list`.

## CLI (`nuage`)

The CLI is the interface of the headless mode and a scripting tool on the desktop. It must be possible for **an agent** (e.g. Claude Code) **to install and use it end to end** in an SSH session.

**Rules:**

- **Never a mandatory interactive prompt**: everything goes through arguments. Confirmations become `--yes`. If standard input is not a terminal and a confirmation is missing, the command fails with a message that names the option to add.
- **`--json` on every command**: stable and documented output, for scripts. Without `--json`, readable output, without color if the output is not a terminal.
- **Documented exit codes**: `0` success, `1` error, `2` incorrect usage, `3` daemon unreachable, `4` authentication required (full list in `docs/CLI.md`).
- Accounts are referred to by their name; remote paths are written `<account>:<path>`.
- Daemon missing: clear message with the command to start it (`systemctl --user start nuaged`).

**Commands (v1):**

```
nuage status [--watch]                         overall status, per account, uploads, quota, cache
nuage account add <provider> [--name N] [--mount-point P]
                     [--token JSON | --token-file F] [--client-id ID --client-secret S] [--no-browser]
nuage account list | rename <account> <name> | reconnect <account> | remove <account> --yes [--force]
nuage mount <account> | unmount <account>
nuage pause [<account>] | resume [<account>]
nuage refresh [<account>]
nuage ls | mkdir | mv | cp | rm | upload | download | link   operations on <account>:<path>
nuage exclude add <account> (<path> | --glob <pattern>) | remove | list
nuage config get | set <key> <value>                     e.g. cache limit, cache age
nuage daemon status | stop
nuage pin add | remove | list                             (v1.1)
```

## Interface (GNOME / libadwaita)

Reminder: [minimalist interface](#principles). Default values never asked during setup: cache limit (20 GB), cache age, offline check interval, exclusions.

### Setup (first launch and "Add account")

Goal: **account connected and sync started without leaving the window, with the minimum number of screens**: 4 for a first Google account, 3 after that or for a provider that does not need an OAuth client. A single window (`AdwNavigationView`, back button at every step).

1. **Choose a provider**: the list of providers in the registry (icon + name). A click moves to the next step. Nothing else on the screen.
2. **Create your Google client** (first Google account only):
   - short, numbered steps: create a project, enable the Drive API, configure the consent screen and **publish it in Production**, create "Desktop app" credentials; each step has a button that opens the right page of the GCP console;
   - two fields, "Client ID" and "Client secret"; the "Continue" button stays disabled as long as they do not have the right format;
   - the client is remembered: subsequent Google accounts skip this screen (can be changed in the settings).
3. **Sign in**:
   - the browser opens automatically (rclone OAuth with the user's client);
   - the screen shows a spinner "Waiting for sign-in in your browser…", a "Cancel" button and a "Copy sign-in link" link in case the browser does not open;
   - it moves to the next step automatically as soon as the token is received;
   - if Google refuses the client (wrong credentials, project in *Testing*), back to step 2 with a message that says what to fix;
   - for a credential-based provider (`AuthSpec::Credentials`), this screen is the form.
4. **Start the sync**:
   - **Folder**: an `AdwActionRow` shows the prefilled folder (`~/Cloud/<account-name>`) with a "Choose…" button (`gtk::FileDialog`). Validations from [Mounting and mount folder](#mounting-and-mount-folder); error shown below the row; main button disabled as long as the folder is invalid.
   - **"Start automatically at login"** (`AdwSwitchRow`), **on by default**. It triggers the request to the Background portal; if the portal refuses, the switch turns off and a toast explains why. It is a global setting (not per account), shown only when adding the first account, and can be changed later in the settings.
   - Main button **"Start sync"** (`suggested-action`): creates the folder if needed and mounts the account; meanwhile it shows a spinner ("Setting up the folder…") and the form is locked. The window stays open and goes back to the list of accounts, with a toast "Google Drive is ready in ~/Cloud/Google Drive" and its "Open Folder" action; if the window is no longer in the foreground, the same thing is also sent as a notification. (Until polish 1, the window closed: the setup seemed to have failed.)

On later launches, if at least one account exists, the app launched by the user opens the main window; at login only the agent (`nuage-tray`) starts, in the system tray, with the window closed (decided at milestone 8). Closing the window ends `nuage-gtk`; the agent stays (decided 2026-11-11, to keep GTK out of memory).

### Main window

Minimal: the list of accounts (icon, name, ✓/↻/⚠/⏸ status, quota bar, "Open folder" and settings buttons) and a "+" button to add an account. The browser opens from an account; it is never shown by default. The settings are in the main menu. Closing the window does not stop the sync.

### System tray icon menu

- Overall status
- Per account: quota, local cache size, offline size (v1.1)
- "Open Nuage" (main window)
- "Open folder" (mounted folder)
- "Refresh"
- "Pause uploads" / "Resume uploads"
- "Preferences"
- "Add account"
- "Quit"

On Ubuntu, the AppIndicator extension is enabled by default and the icon is shown. On a GNOME without the extension, the main window must be enough to do everything.

### Quit

"Quit" stops the app and the daemon (unmounting, then stopping rclone). If uploads are still pending, the behavior is set in the settings ("When quitting with pending uploads"):

| Option | Effect |
|---|---|
| **Finish in background** (default) | the window and the icon disappear; the daemon finishes the uploads, then unmounts and stops |
| Ask | "N files are still uploading" dialog: Wait / Quit now |
| Quit now | immediate stop; the uploads stay in the rclone cache and resume at the next launch |

In the CLI, `nuage daemon stop` applies the same setting; `--now` forces an immediate stop.

### Settings (`AdwPreferencesDialog`)

- Global (Preferences, in the main menu): autostart, "Quit" behavior, OAuth client, offline check interval (v1.1), offline space warning threshold (v1.1).
- Per account (its own dialog, from the gear button of its row in the main window): name, mount folder, cache limit, maximum cache age, exclusions, pinned items (v1.1), account removal.

### Notifications (Notification portal)

Upload error, expired token, full cache, offline conflict (v1.1). In headless mode, these events appear in `nuage status` and in the service journal (`journalctl --user -u nuaged`).

## Security and storage

- `rclone.conf` is encrypted with `RCLONE_CONFIG_PASS`. The password is stored in the keyring: Secret portal (`ashpd`) in the Flatpak, Secret Service (`org.freedesktop.secrets`) in native mode if it is available.
- **Headless without a keyring**: `rclone.conf` is not encrypted but protected with `0600` (like rclone by default). `nuage status` reports it and the README explains it.
- rclone obscures passwords (WebDAV, Nextcloud, Proton) in a reversible way, but not S3 secret keys or OAuth tokens: only the encryption of `rclone.conf` really protects them.
- rc API on 127.0.0.1, random port and credentials at every launch, never `--rc-no-auth`.
- IPC socket with `0600`, accessible to the user only.
- No token or password in the logs (`tracing` filter).

**Locations** (Flatpak: `~/.var/app/io.github.felixilgatto.Nuage/{config,cache,data}`; native: XDG directories `~/.config/nuage`, `~/.cache/nuage`, `~/.local/share/nuage`):

| Contents | Subfolder | Version |
|---|---|---|
| rclone config (`RCLONE_CONFIG`) | `config/rclone/rclone.conf` (native: `~/.config/nuage/rclone.conf`) | v1 |
| App config (TOML: accounts, exclusions, settings) | `config/nuage/` (native: `~/.config/nuage/`) | v1 |
| VFS cache (`--cache-dir`) | `cache/rclone` | v1 |
| Hidden rclone mounts | `data/mounts/<account>` | v1 |
| Pin store | `data/pins/<account>/…` | v1.1 |
| Excluded files kept locally | `data/local-only/<account>/…` | v1 |
| Offline metadata (SQLite) | `data/` | v1.1 |

## Tech stack

| Role | Crate | Note |
|---|---|---|
| UI | `gtk4`, `libadwaita` (gtk-rs), without relm4 | GObject subclasses, `.ui` or Blueprint files |
| Tray | `ksni` | StatusNotifierItem + dbusmenu in pure Rust (GTK4 has no AppIndicator) |
| Portals | `ashpd` | Background, OpenURI, Notification, Secret, FileChooser |
| CLI | `clap` | subcommands, `--json` |
| IPC | `tokio` (`UnixListener`), `serde_json` | newline-delimited JSON |
| HTTP to rc | `reqwest` (rustls) | |
| Async | `tokio`; on the GTK side, `async-channel` to the GLib loop | never block the GTK thread |
| Serialization | `serde`, `serde_json`, `toml` | |
| Overlay FUSE (`nuage-fs`) | `fuser` | v1 |
| Offline metadata (v1.1) | `rusqlite` (`bundled` feature) | |
| i18n | `gettext-rs` | strings in English, ready to translate |
| Errors | `thiserror` (crates), `anyhow` (binaries) | |
| Logs | `tracing` | journald when launched by systemd |
| `.deb` package | `cargo-deb` | |

## Packaging

### Flatpak (desktop)

YAML manifest in `build-aux/flatpak/`:

- runtime `org.gnome.Platform` (latest stable version);
- extension `org.freedesktop.Sdk.Extension.rust-stable`;
- Cargo dependencies vendored with `flatpak-cargo-generator` (`cargo-sources.json`), offline build;
- `command: nuage-gtk`.

Modules:

1. `rclone`: official binary with pinned sha256 (or Go build via the golang extension), installed in `/app/bin`.
2. `fusermount-wrapper`: script installed as `/app/bin/fusermount3`.
3. `nuage`: `cargo build --release --offline` (`nuaged`, `nuage`, `nuage-gtk`, `nuage-tray`), `.desktop`, icon, `metainfo.xml`.

`finish-args` (minimal):

```yaml
- --socket=wayland
- --socket=fallback-x11
- --device=dri
- --share=ipc
- --share=network
- --talk-name=org.freedesktop.Flatpak        # fusermount3 wrapper only
- --talk-name=org.kde.StatusNotifierWatcher  # system tray icon
- --talk-name=org.freedesktop.secrets        # fallback if the Secret portal is missing
- --filesystem=home                          # freely chosen mount folders
```

Never `--filesystem=host`.

### Native (headless)

- **Tarball** `nuage-<version>-x86_64-linux.tar.gz`: `nuaged`, `nuage`, `rclone` (same pinned version as the Flatpak), `nuaged.service` (systemd user unit), `install.sh` script that installs into `~/.local` without `sudo`.
- **`.deb`** (`cargo-deb`): same binaries in `/usr/bin` (rclone in `/usr/lib/nuage/` so as not to conflict with apt's rclone), unit in `/usr/lib/systemd/user/`, `fuse3` dependency.
- Binaries statically linked as much as possible (musl to be evaluated), without any GTK dependency.
- Typical installation, documented in the README:

```
sudo apt install ./nuage_<version>_amd64.deb        # or: tar xf … && ./install.sh
systemctl --user enable --now nuaged
loginctl enable-linger "$USER"                         # to run without an open session
nuage account add gdrive --token-file token.json --mount-point ~/Cloud/Drive
nuage status
```

## CI (GitHub Actions)

Workflows in `.github/workflows/`, on the `felixilgatto/nuage` repository. **No cloud credentials in CI**: tests use the `local`/`memory` provider.

### `ci.yml` — on every push and pull request

**`check` job** (`ubuntu-24.04` runner with `libgtk-4-dev libadwaita-1-dev`, or GNOME SDK container to have the same versions as the Flatpak):

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace`: unit tests + integration tests against a real `rclone rcd` (same version as in the packages) with the `local` provider: account creation, FUSE mount (available on Ubuntu runners), `vfs/stats`, pause/resume, browsing, file operations, exclusions; in v1.1, offline scenarios of `nuage-fs`.
- **CLI end-to-end tests**: start `nuaged`, drive a full scenario with `nuage … --json` (adding a `local` account, mounting, writing, `status`, pause, exclusion, removal), check JSON outputs and exit codes.
- `cargo deny check` (MIT-compatible licenses, security advisories, duplicates) via `EmbarkStudios/cargo-deny-action`.
- Dependency rules between crates: script on `cargo metadata` that fails if the [dependency table](#cargo-workspace) is not respected, or if `daemon`/`cli` pull in a GTK dependency.
- Cargo cache: `Swatinem/rust-cache`.

**`native` job**: build of `nuaged` and `nuage` without GTK on a minimal Ubuntu image, production of the tarball and the `.deb` as artifacts, then installation of the `.deb` in a clean container and smoke test (`nuage --version`, daemon startup, `nuage status --json`).

**`flatpak` job** (container `ghcr.io/flathub-infra/flatpak-github-actions:gnome-<version>`, action `flatpak/flatpak-github-actions/flatpak-builder`):

- checks that `cargo-sources.json` is up to date (regeneration then `git diff --exit-code`);
- builds the manifest and publishes the `nuage.flatpak` bundle as an artifact;
- runs `flatpak-builder-lint` on the manifest and the built repository (AppStream, permissions). In v1, the warnings about `org.freedesktop.Flatpak` and `home` are tolerated and documented.

### `release.yml` — on `v*` tags

- Reruns the Flatpak and native builds.
- Publishes a GitHub Release with the `.flatpak` bundle, the tarball, the `.deb`, their sha256 sums and the changelog extracted from `CHANGELOG.md`.

### Miscellaneous

- `dependabot.yml`: weekly `cargo` and `github-actions` updates.
- Pinned actions (major tag at least, SHA ideally).
- Minimal `GITHUB_TOKEN`: `contents: read` everywhere, `contents: write` only in `release.yml`.

## Documentation

### README.md (in English, for a non-technical audience)

1. **Overview**: one sentence, screenshot, CI and license badges.
2. **Features**: status icon, quota, unified browser, mount folder per cloud, cache limit, pause, exclusions, headless mode, offline.
3. **Providers**: table of provider / status (available, preview, planned) / capabilities (quota, share link, trash).
4. **Installing on the desktop**: from the Flatpak repository on GitHub Pages (`flatpak install --user https://felixilgatto.github.io/nuage/nuage.flatpakref`, updates with `flatpak update`) or a bundle from the release, `fuse3` prerequisite.
5. **Installing on a server (headless)**: `.deb` or tarball, systemd service, `enable-linger`, connecting an account by SSH tunnel or by token, full end-to-end example.
6. **Building from source**: `flatpak-builder`, and development outside Flatpak (`cargo run`, apt packages).
7. **First launch**: adding an account, choosing the folder, cache limit.
8. **Creating your own Google OAuth client** (required for Google Drive) step by step, with screenshots: GCP project, Drive API, consent screen in *Production*, "Desktop app" credentials.
9. **Flatpak permissions**: each `finish-arg` and its reason, in particular `org.freedesktop.Flatpak` (FUSE mount) and `home`.
10. **Offline mode**: pinning, what works without network, conflicts, space used.
11. **Troubleshooting**: missing icon (AppIndicator extension), mount that fails (`fuse3`, orphan mount), folder outside the home (`flatpak override`), expired token, unreachable daemon.
12. **Architecture**: short diagram, links to `docs/ARCHITECTURE.md`, `docs/CLI.md` and `docs/ADDING_A_PROVIDER.md`.
13. **Contributing** and **License** (MIT; rclone is bundled under its own MIT license).

### Other documents

- `docs/ARCHITECTURE.md`: processes, crates, dependency rules, IPC protocol, lifecycle of the daemon and of rclone.
- `docs/CLI.md`: all commands, options, JSON format of each output, exit codes.
- `docs/ADDING_A_PROVIDER.md`: module in `providers`, Cargo feature, registration, icon, tests, row in the README table.
- `CHANGELOG.md`: used by `release.yml`.

## Milestones

| # | Milestone | Deliverable |
|---|---|---|
| 1 | **Proof of the hard part** | Minimal Flatpak (rclone + wrapper) that mounts a hand-configured Drive remote in a folder of the home, visible in Nautilus. Also check the points of [To verify](#to-verify) marked "milestone 1". **If it fails, we stop and reassess.** **Validated on 2026-10-31**: Google Drive mounted from the Flatpak with a personal OAuth client, via the host-side relay (see [hard part](#the-hard-part-fuse-in-the-flatpak-sandbox)); browsing, reading and `link.html` shortcuts checked. |
| 2 | **Skeleton** | Complete Cargo workspace (including `ipc`, `daemon`, `cli`), crates with their interfaces, `local` provider, green `ci.yml`, initial README, `LICENSE`. |
| 3 | **Daemon and CLI** | `RcdSupervisor`, typed rc client, `MountManager`, `UploadControl`, `StatusPoller`, IPC protocol, CLI (`status`, `account`, `mount`, `pause`, `config`), end-to-end tests with `local`. |
| 4 | **`nuage-fs`: relay** | Overlay file system as a simple relay to the hidden rclone mount, without regression; `MountManager` mounts both layers; `nuage-fuse-relay` replaces the prototype's Python relay; FS test suite. |
| 5 | **Google Drive** | `gdrive` provider (own client required, `link.html` web shortcuts), `nuage account add gdrive` with both headless OAuth methods, encrypted or `0600` config. |
| 6 | **Native packages** | systemd unit, tarball, `.deb`, `native` CI job. **Nuage is usable headless from here on.** |
| 7 | **Exclusions** | rclone filters on the hidden mount + local interception in `nuage-fs` (`local-only` folder), remounting only the hidden mount, `nuage exclude` CLI. |
| 8 | **GTK app and tray** | `nuage-gtk`: guided setup (Google client screen included), main window, aggregated status icon and menu, autostart (Background portal). |
| 9 | **Browser** | Graphical browser (browsing, actions, badges, drag and drop) and file operations from the CLI. |
| 10 | **Settings and integration** | Settings, "Quit" behavior, notifications, mount folder per account, exclusions in the GUI. |
| 11 | **Finishing touches → v1** | `flatpak` CI job, `release.yml`, AppStream, icon, complete README and `docs/CLI.md`. |
| 12 | **v1.1 — Offline** | Pinning in `nuage-fs`: SQLite metadata, pin store, uploading back, conflicts, offline startup, GUI + CLI. |
| 13 | **v2** | OneDrive, shared drives; Flathub postponed (see below). |
| 14 | **Providers** | Dropbox, pCloud, Nextcloud, WebDAV, S3 (bucket as the account root); Proton Drive in preview; sign-in check for credential-based accounts. Nextcloud, WebDAV and S3 checked against Nextcloud 31, hacdias/webdav and MinIO in containers; Dropbox, pCloud and Proton remain to be tried with real accounts. Done on 2026-11-05. |

**Milestone 15 — Distribution (2026-11-05).** The release workflow builds for x86_64 and aarch64: `.deb`, `.rpm` (Fedora, openSUSE) and tarball of static binaries, smoke-tested in clean Ubuntu, Debian and Fedora containers; Flatpak bundles; and a Flatpak repository signed with the project's GPG key, published on GitHub Pages with a `.flatpakref`, from which installs and bundles update (`flatpak update`). Downloads carry checksums and GitHub build provenance (`gh attestation verify`). `scripts/prepare-release.sh` prepares a version, `scripts/check-release.sh` checks that the tag, `Cargo.toml`, the metainfo and the changelog agree. A manual run of the workflow tests all of it without publishing. Publishing (the tag) is still the maintainer's decision.

**Decided after milestone 13 (2026-11-01).** No publication for now: the user uses Nuage daily, we polish it, then the maintainer decides. Flathub is prepared (`docs/FLATHUB.md`) but postponed (`org.freedesktop.Flatpak` permission to justify, no Flathub by default on Ubuntu); a Flatpak repository on GitHub Pages is the intermediate step considered if automatic updates matter.

**Option to decide later: AppImage.** Outside the sandbox, it mounts directly with the host's `fusermount3` (neither wrapper nor relay). Hard parts: static type 2 runtime so as not to depend on `libfuse2` (absent from Ubuntu by default); the daemon must outlive the app, so `nuaged`, `nuage` and rclone are copied into `~/.local/share/nuage/bin/` on first launch (and kept up to date); GTK 4 and libadwaita bundled (80 to 120 MB deliverable); updates via the zsync information of the GitHub releases. Possible dedicated milestone: build in CI, startup test on Ubuntu 24.04 and Fedora, file attached to the releases.

## Acceptance criteria

### v1

**CI and release**

- On a PR, the CI is green: fmt, clippy, tests (including integration with a real rclone and the `local` provider), CLI end-to-end tests, cargo-deny, dependency rules, Flatpak and native builds with artifacts.
- A `v0.1.0` tag produces a GitHub Release with the `.flatpak` bundle, the tarball, the `.deb` and their sha256 sums.

**Desktop: setup**

- On a fresh Ubuntu with flatpak: installing the bundle, then adding the Drive account entirely from the interface, auth in the browser, **in 4 screens** (provider, guided Google client, authentication, folder + autostart). A user who follows the guided screen creates their GCP client without consulting any other documentation. A second Google account only takes 3 screens.
- No other question is asked during setup.
- After "Start sync", the window shows the list of accounts with "Open Folder", the icon is in the system tray and the folder is mounted.
- With autostart on, the icon appears at the next login without manual action.

**Headless**

- On a freshly installed Ubuntu server, without a graphical environment, reachable over SSH: installing the `.deb` or the tarball, connecting a Drive account (by SSH tunnel or by `--token-file`), working mount and `nuage status`, **without any interactive prompt** when all options are provided.
- An agent (e.g. Claude Code) carries out this scenario end to end by following only the README.
- With `enable-linger`, the folder is remounted after a server restart without any user logging in.
- Every command accepts `--json` and returns the documented exit codes.

**Mounting, sync and cache**

- The chosen folder appears in Nautilus and files open in the host's apps. Changing the folder unmounts the old one and mounts the new one.
- Copying a large file into the mounted folder: the icon switches to ↻ with the number of files, the file carries the "Uploading" badge in the browser, then everything goes back to ✓ (`nuage status` follows the same states).
- When paused, a file copied into the folder is not uploaded; it is sent on resume. The pause survives a daemon restart.
- A Google Doc appears in the mounted folder as a shortcut that opens in the web browser.
- The quota shown matches the one on drive.google.com (up to rounding).
- With a 1 GB cache limit, the account's cache does not stay above 1 GB after reading several GB.

**Browser and modularity**

- The browser has the same design for `gdrive` and `local`; an unsupported action is hidden, not an error.
- Adding a declarative provider (e.g. OneDrive) modifies no file in `engine`, `ui`, `tray` or `cli`. S3 once required extending the abstraction (`Account.root`, `Provider::root`, sign-in check); the following providers benefit from it without touching the engine.

**Exclusions**

- Excluding a folder from the browser or the CLI: after remounting, it disappears from the mounted folder and is no longer downloaded; it stays visible and greyed out as "Not synced" in the browser and can be included again.
- A `*.tmp` file is never uploaded.

**Robustness and security**

- Network cut off: explicit status, no crash; back to normal afterwards.
- "Quit" with pending uploads (default setting): the interface disappears, the uploads finish, then the folder is unmounted.
- After a `kill -9` of the daemon, the next launch cleans up the orphan mount.
- Removing an account erases its local data (remote, cache, empty folder) and touches nothing in the cloud.
- No secret in the logs. On the desktop, no secret in clear text on disk; headless without a keyring, `rclone.conf` is `0600` and `nuage status` reports it.

### v1.1 (offline)

- A pinned file opens **at its usual path** in the mounted folder, with the network cut off.
- After a **restart without network**, the mounted folder appears, the tree is browsable and pinned files open.
- A pinned file modified offline is uploaded automatically when the network is back.
- A simultaneous local and remote change produces two files (no loss) and a notification.
- A new file added remotely in a pinned folder is downloaded automatically.
- Filling the VFS cache beyond its limit never evicts a pinned file.
- Unpinning a file whose changes have not been uploaded is refused (or asks for confirmation) as long as they are not sent.
- Offline, unpinned items are visible, greyed out, and return a clean error when opened, without blocking Nautilus.
- All of this can also be driven from the CLI (`nuage pin …`).
- The CI covers these cases with the `local` provider, by simulating the network cut (stopping the underlying rclone mount).

## To verify

| Point | When |
|---|---|
| The FileChooser portal does return the real path (not `/run/user/*/doc/`) for a folder under the home with `--filesystem=home` | milestone 1 |
| Exact names of rc fields and options (`vfs/stats`, `vfsOpt`, etc.) on the bundled version | milestone 3, then at every version upgrade (integration test) |
| The "URL + SSH tunnel" flow with `config_auth_no_browser` works end to end with a real Google account | milestone 5 |
| Static build (musl) of the native binaries with rclone and FUSE | milestone 6 |
| Opening `link.html` shortcuts: the default HTML application may not be a browser (Thunderbird on the test machine). Should Nuage open the URL in the default browser itself? | milestone 5 |
| `nuage-fuse-relay` run on the host from the app's path (`/.flatpak-info`), `--user` and system installation | milestone 4 |
| Whether the relay is still needed on other distributions (Fedora, Debian, Arch); keep it in all cases if it has no side effects | milestone 4 |
| Performance of `nuage-fs` as a relay (throughput, listing latency) compared with a direct rclone mount | milestone 4 |
| rclone has not added native pinning in the meantime (otherwise, reassess pinning in `nuage-fs`) | before milestone 12 |

## Open questions

- **aarch64** (Raspberry Pi, ARM servers): done on 2026-11-05 (milestone 15), native packages and Flatpak, built on GitHub's ARM runners.
