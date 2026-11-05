# Nuage architecture

Nuage is an interface on top of rclone's remote control API (rc). rclone does
the mounting, caching, OAuth and transfers; Nuage does not reimplement a sync
engine. The one exception is `nuage-fs`, an overlay filesystem for exact
exclusions and, in v1.1, offline pinning.

> Status: milestone 14. The daemon supervises rclone, signs in to the
> providers (browser or form), mounts each account in two layers (`nuage-fs`
> over a hidden rclone mount), applies exclusions, pauses uploads, reports the status and runs
> file operations; the CLI and the desktop application (setup, main window,
> tray, file browser) drive all of it. Offline pinning (v1.1) is in.

## Processes

```
nuaged  (daemon: engine + rclone rcd)
   ▲  Unix socket, newline-delimited JSON (Command / Event)
   ├─ nuage-gtk   desktop app: window + tray icon
   └─ nuage       CLI (humans, scripts, agents)
```

- The daemon owns all the state: accounts, mounts, exclusions, pins. Clients
  never talk to rclone.
- One daemon per user. A second `nuaged` finds the first through the socket
  and exits; a stale socket left by a crash is replaced.
- Everything the GUI does goes through a `Command`, so everything (except the
  graphical browser) is also possible from the CLI.

## Crates

| Directory | Package | Role |
|---|---|---|
| `crates/core` | `nuage-core` | domain model, `Capabilities`, `AuthSpec`, `Command` / `Reply` / `Event`, errors |
| `crates/ipc` | `nuage-ipc` | socket protocol: framing, handshake, async client and server |
| `crates/rclone-rc` | `nuage-rclone` | typed rc client, `RcdSupervisor` |
| `crates/providers` | `nuage-providers` | `Provider` trait, `ProviderRegistry`, one module and Cargo feature per provider |
| `crates/engine` | `nuage-engine` | accounts, mounts, uploads, exclusions, status, files, pins |
| `crates/fs` | `nuage-fs` | overlay FUSE filesystem: relay to the hidden rclone mount |
| `crates/fuse-relay` | `nuage-fuse-relay` | binary run on the host by the Flatpak's `fusermount3` wrapper |
| `crates/daemon` | `nuaged` | binary: engine + IPC server |
| `crates/cli` | `nuage` | binary: CLI (clap), IPC client |
| `crates/tray` | `nuage-tray` | StatusNotifierItem icon (ksni) |
| `crates/ui` | `nuage-ui` | GTK 4 + libadwaita interface |
| `crates/gtk` | `nuage-gtk` | binary: assembles ui + tray |

### Dependency rules

Checked in CI by `scripts/check-deps.sh`:

| Crate | May depend on |
|---|---|
| `core` | no project crate |
| `ipc`, `providers`, `rclone-rc`, `fs` | `core` |
| `engine` | `core`, `rclone-rc`, `providers`, `fs` |
| `daemon` | `engine`, `ipc`, `core` |
| `cli`, `ui`, `tray` | `core`, `ipc` — never `engine` or `rclone-rc` |
| `gtk` | `core`, `ipc`, `ui`, `tray` |
| `fuse-relay` | no project crate |

`nuaged` and `nuage` must not pull any GTK crate (checked with `cargo tree`),
so the headless package installs on a bare server.

## Providers

A provider is mostly declarative: rclone backend, `AuthSpec`, configuration
parameters, static `Capabilities`, recommended VFS options, native-document
policy, and the folder of the remote an account shows (`root()`, e.g. an S3
bucket; `Account.root`). Real capabilities are the static ones merged with rclone's
`operations/fsinfo` features. Interfaces hide actions according to
capabilities, never according to the provider name. Adding a provider means
adding a module and a Cargo feature in `providers`, without touching
`engine`, `ui`, `tray` or `cli`.

## IPC protocol

- Socket: `$XDG_RUNTIME_DIR/nuage/nuaged.sock` (mode `0600`, directory
  `0700`). Inside the Flatpak: `$XDG_RUNTIME_DIR/app/io.github.felixilgatto.Nuage/nuaged.sock`,
  which the host can also reach; the CLI tries both. `NUAGE_SOCKET` overrides.
- Framing: one JSON object per line.
- Handshake: the client sends `{"type":"hello","protocol_version":2,"client":"…"}`;
  the daemon answers `welcome`, or `rejected` with a message explaining which
  side to update, then closes.
- Requests: `{"type":"request","id":N,"command":{…}}` → `{"type":"response","id":N,"ok":{…}}`
  or `{"type":"response","id":N,"error":{"code":"…","message":"…"}}`.
- Events: after `{"type":"subscribe"}` and the `subscribed` acknowledgement,
  the daemon pushes `{"type":"event","event":{…}}` lines.

## Daemon and rclone lifecycle

- rclone runs as a child process (`rclone rcd` on 127.0.0.1, random port and
  credentials, never `--rc-no-auth`), supervised by `RcdSupervisor`.
- Startup: clean up orphan mounts left by a crash, then remount accounts.
- Shutdown (`SIGTERM`, `nuage daemon stop`, "Quit"): unmount every account,
  then `core/quit`.
- rcd's PID is written to `data/rcd.pid`. An rcd left by a daemon killed
  with `SIGKILL` is stopped at the next start, then any FUSE mount still
  present at an account folder is detached (`fusermount3 -u -z`).
- If rcd dies, the engine restarts it (at most every 5 s) and remounts.
- Mounts: see [Mounts](#mounts).

## Mounts

```
~/Cloud/Google Drive            ← nuage-fs (fuse.nuage), served by nuaged
   └─ everything                → relayed to the hidden rclone mount
data/mounts/<account id>        ← rclone (fuse.rclone), VFS cache, write-back
```

- `MountPlan::layered` gives both paths. Mounting does rclone first, then
  `nuage-fs`; unmounting does `nuage-fs` first, then rclone.
- `nuage-fs` (`crates/fs`, on `fuser`) gives out its own inode numbers,
  mapped to paths relative to the hidden mount, and relays every operation
  to it. It is served by 8 threads, so a slow read (rclone downloading) does
  not block listings or other files.
- **If rclone dies**, the user's folder stays mounted. The engine closes
  `nuage-fs`'s gate: requests wait (up to 10 s) instead of seeing an empty
  folder. Errors of the dead mount (`ENOTCONN`, `ECONNABORTED`) are retried
  the same way, and files open at that moment are reopened by path. Once
  rcd is restarted and the hidden mount is back, the gate opens again.
- Both layers mount through `fusermount3` with the `_FUSE_COMMFD` socket,
  as rclone does (`fuser` would open `/dev/fuse` directly, which the
  sandbox does not allow). The threads share one `/dev/fuse` descriptor for
  the same reason.
- Unmounting a busy folder (open in Files, a shell inside it): `fusermount3
  -u -z` detaches it, then the session is ended through
  `/sys/fs/fuse/connections/<id>/abort`, and the engine waits at most 5 s
  for the serving threads. Without it, the kernel would keep the session
  (and the daemon's stop) waiting for the last user of the folder.
- Orphans: a `SIGKILL`ed daemon leaves `fuse.nuage` and `fuse.rclone`
  mounts; the next start detaches them, except user folders it still serves.

### Flatpak: mounting from the sandbox

The sandbox has neither `fusermount3` nor the right to mount.
`/app/bin/fusermount3` is `build-aux/flatpak/fusermount-wrapper.sh`:

1. libfuse (rclone) or `nuage-fs` run it with a socket in `$_FUSE_COMMFD`.
2. It runs `nuage-fuse-relay` on the host with `flatpak-spawn --host
   --forward-fd`, from the application's host path (`app-path` in
   `/.flatpak-info`), through the deploy's `active` link: an update removes
   the running version's directory, and the relay of the new version is
   used until Nuage restarts.
3. The relay creates its own socket pair, runs the host's `fusermount3`
   with it (Ubuntu 26.04's AppArmor profile refuses sockets inherited from
   the sandbox), receives the `/dev/fuse` descriptor and passes it back on
   the forwarded socket.

Unmounts go straight to the host's `fusermount3`. The relay is built as a
static executable: it runs on the host, whose glibc may be older than the
runtime's. It refuses to run itself in a loop (a wrapper found again in
`PATH`).

The mounts made by the host propagate into the sandbox, so `nuage-fs` in
the sandbox reads the hidden rclone mount, and host applications see the
user's folder.

## Engine

One `Engine` is shared by all IPC connections. Its state (accounts, pause
flags, settings, what is mounted, last status) sits behind one mutex. A
background task ticks every second:

1. restart rcd if it died;
2. release uploads (see below);
3. every other tick, poll `vfs/stats` (and `operations/about` every 5
   minutes) and publish events for what changed (`StatusChanged`,
   `AccountStateChanged`, `QuotaUpdated`, `CacheFull`, `MountChanged`).

`nuage status` triggers a poll, so it never shows stale data.

### Accounts

- An account is a record in `config.toml` plus a remote `nuage-<id>` in
  `rclone.conf`. The id is random and stable; users refer to accounts by
  name (case-insensitive), which can change.
- `account add` creates the remote with `config/create` (`nonInteractive`),
  mounts it, and rolls everything back if a step fails. For a
  `Credentials` provider, rclone saves the form without trying it, so the
  engine lists the account root first (30 s at most) and turns a failure
  into what to fix.
- An account's fs is `<remote>:<root>` (`mounts::remote_fs`), `root` being
  empty for most providers; folders below it are built with
  `mounts::join_fs`. The `local` provider is an rclone `alias` of a folder,
  so its root is `<remote>:`. Two `local` accounts over the same folder share one
  rclone VFS (and upload queue): avoid it.
- `account remove` unmounts, deletes the remote and the VFS cache (paths
  reported by `vfs/stats`), and the mount folder if empty. Refused while
  uploads are pending, unless `--force`.

### Signing in (OAuth)

1. `account add` (or `account reconnect`) creates (or updates) the remote
   with `config_auth_no_browser`: rclone stops at its question "use a web
   browser?" (`config_is_local`).
2. Answering "yes" as a background rc job starts rclone's OAuth server on
   `127.0.0.1:53682`. The daemon replies `SignIn { session, url }` with the
   page from `config/oauthstatus`.
3. The client shows the link (and opens it when a browser is available),
   then sends `SignInWait`. On a server the user reaches the page through
   `ssh -L 53682:localhost:53682`.
4. When the job ends, the account is saved and mounted (or remounted to use
   the new token). `SignInCancel` (Ctrl-C in the CLI) stops the OAuth server
   with `config/oauthstop` and deletes the new remote.

One sign-in at a time (rclone has one OAuth server); an abandoned one is
cancelled after 10 minutes. With `--token`, no browser is involved: one
`config/create` (or `config/update`) with `config_refresh_token=false`.
A second account of a provider reuses the OAuth client of the first one
(read back with `config/get`).

### Secrets

`rclone.conf` holds the tokens. At startup the daemon looks for a password
in the keyring (`oo7`: the Secret portal in the Flatpak, the Secret Service
natively), creating a random one if needed, encrypts a plain
`rclone.conf` once (`rclone config encryption set --password-command`), and
passes it to rcd as `RCLONE_CONFIG_PASS`. Without a keyring (headless, or
`NUAGE_KEYRING=off`), the file stays plain in mode `0600` and
`StatusReport.config_encrypted` is `false`. An encrypted file whose keyring
does not answer (at login, the portal or the keyring may not be ready, or
the keyring waits to be unlocked) does not stop the daemon: it serves
clients and tries again every 5 s (each try waiting up to 2 min for an unlock
prompt). In the meantime the accounts are `waiting_for_keyring` (a key in
the tray, not a warning); after 3 min without an answer they turn to `error`
with the reason. Commands that need rclone answer with that reason. With `NUAGE_KEYRING=off` the daemon stops
with a message. The window starts a daemon that crashed again, at most once
a minute, and keeps the previous log as `nuaged.log.old`. The password is never logged
(`RcdSupervisor`'s `Debug` hides it).

### Exclusions

Per account, in `config.toml`: user rules (`Path` relative to the account
root, or `Glob`) and the default patterns (`~$*`, `.~lock.*#`, `*.tmp`,
`.goutputstream-*`, on unless `<account>:default-exclusions off`).
`nuage_core::ExclusionSet` matches paths with rclone's filter semantics
(a pattern without `/` matches a name at any depth; everything inside an
excluded folder is excluded) and produces the rclone rules (`- <rule>`,
`- <rule>/**`). Two layers apply them:

- **rclone filters** on the hidden mount (`_filter.FilterRule` of
  `mount/mount`): excluded remote items are neither listed nor downloaded.
- **`nuage-fs`**: an excluded path always lives in
  `data/local-only/<account>`, a non-excluded one on the hidden mount.
  Listings merge both; renaming a file between them copies it (atomic saves
  write an excluded temporary file and rename it to its final name). rclone
  alone would upload an excluded file created locally.

A change is given to `nuage-fs` at once (files no longer excluded move to
rclone, and so to the cloud), then only the hidden mount is remounted
(`remount_hidden`): the user's folder stays, requests wait meanwhile.
Refused while uploads are pending unless `force` (`--yes`). Removing an
account with local-only files needs `--force`.

### Pause

Every mount uses `WriteBack=8760h`, so rclone never uploads by itself.
Each tick, for mounted accounts that are not paused, the engine reads
`vfs/queue` and releases (`vfs/queue-set-expiry`, `expiry=-1e9`) items held
by that delay once they are 2 s old. Items with a short delay are rclone's
own retries and are left alone. Pausing holds again items released but not
started. The pause flag is in `config.toml`; held writes survive a restart
and are queued again at the next mount.

### State

| Condition | `SyncState` |
|---|---|
| rclone waits for the keyring, less than 3 min | `waiting_for_keyring` |
| rclone waits for the keyring, longer | `error` (with the reason) |
| last mount failed | `error` (with the reason) |
| not mounted | `stopped` |
| upload errors, or cache disk full | `error` |
| paused | `paused { queued }` |
| uploads in progress or queued | `syncing { uploads, queued }` |
| otherwise | `up_to_date` |

### File operations

`files.rs`: list, mkdir, move, copy, delete, upload, download and share link
through the rc API (`operations/*`, `sync/copy|move`), **independently of the
mount**: they work with the account unmounted. Transfers run as `_async`
jobs polled with `job/status`, so no HTTP timeout cuts them.

- Moving or copying onto an existing folder puts the item inside it; an
  existing file is never overwritten (`already_exists`). Folders cannot go
  inside themselves. Accounts can differ (rclone copies across remotes).
- After a change, the parent folders are forgotten by the account's mount
  (`vfs/forget dir=…`): the mounted folder shows the change within a second
  or two (the kernel caches of both FUSE layers).
- `Entry.badges`: `uploading` (from `vfs/queue`; for a folder, something in
  it; files written but not uploaded yet are listed too), `not_synced`
  (exclusion rules), `online_only` (offline: the folder is listed from the
  mount's directory cache, files without a VFS cache copy cannot open).
- Provider-native documents: `NativeDocPolicy::WebShortcut` declares the
  shortcut suffix (`.link.html`) and the web URL template
  (`https://drive.google.com/open?id={id}`, with the backend ID from
  `operations/list`), so the engine stays generic.

### Files

| Content | Native | Flatpak (`~/.var/app/<id>/`) |
|---|---|---|
| `config.toml` (accounts, pause, settings), mode `0600` | `~/.config/nuage/` | `config/nuage/` |
| `rclone.conf` | `~/.config/nuage/rclone.conf` | `config/rclone/rclone.conf` |
| VFS cache | `~/.cache/nuage/rclone/` | `cache/rclone/` |
| `rcd.pid`, hidden mounts | `~/.local/share/nuage/` | `data/` |

## Desktop application

`nuage-gtk` (crates `gtk`, `ui`, `tray`) is a client of `nuaged` like the
CLI; it starts the daemon if it does not answer (next to its executable,
then `PATH`; log in the user data folder) and never stops it on close.

- `nuage_ui::Backend`: a thread with a tokio runtime runs the IPC client;
  the GTK code awaits its answers through `async-channel`, and receives the
  daemon's events (reconnecting when it restarts).
- Setup (`ui/onboarding.rs`), one window, at most four screens: provider →
  the user's OAuth client (only if the provider requires one and no account
  of it exists) → sign-in in the browser (or the credentials form) → folder
  and "Start automatically at login" (first account) → "Start Sync". The
  account is created with `no_mount`, the folder checked with
  `ValidateMountPoint`, moved with `AccountSetMountPoint`, then mounted.
  Nothing provider-specific is in the interface: the client guide, the client
  ID format and the form come from `ProviderInfo`.
- Main window: accounts with state, quota and "Open folder"; "+"; menu
  (About, Quit). Closing it hides it. Launched by the user, the application
  opens it; `--background` (login) does not when the tray shows.
- File browser (`ui/browser.rs`), opened from an account row, its own
  window: `AdwNavigationSplitView` with the accounts (quota bars) and the
  folder; breadcrumb, filter, new folder, upload, list (`GtkColumnView`,
  sortable) or grid (`GtkGridView`) view, remembered in
  `$XDG_CONFIG_HOME/nuage/gtk.ini`. Context menus show only what the
  account can do (`Capabilities`): open, show in Files, download to, copy
  link, cut/copy/paste (also across accounts), rename, exclude/include,
  move to trash or delete (confirmed without a trash), properties. Files
  dropped from the file manager are uploaded. Opening: folders in the
  browser, native documents in the web browser (`GtkUriLauncher`), files
  from the mounted folder, or from a downloaded copy when unmounted or
  excluded. A folder with uploads in progress is listed again every 3 s.
- Tray (`ksni`, StatusNotifierItem): aggregated state icon; menu with each
  account's state, quota and cache, Open Nuage, Open Folder, Refresh,
  Pause/Resume Uploads, Add Account, Quit. Without a StatusNotifierWatcher
  (GNOME without the AppIndicator extension), the window is shown instead
  and closing it quits the application (the daemon keeps running).
- Start at login (`nuage-gtk --background`), asked at the last setup screen
  of the first account and switched in the preferences: in the Flatpak, the
  Background portal; outside of it (the portal refuses unsandboxed
  applications), an entry in `~/.config/autostart/`.
- Preferences (`ui/preferences.rs`, `AdwPreferencesDialog`, from the main
  menu, Ctrl+, or the tray): start at login, "quit with pending uploads",
  the providers' OAuth clients (`OAuthClientGet` / `OAuthClientSet`: a new
  client signs its accounts out, then they are reconnected one by one);
  per account: name, folder, sign in again, cache limit and age, the
  default patterns and the user's exclusions, removal (with the daemon's
  `busy` answer confirmed before `force`).
- Reconnecting (`ui/reconnect.rs`): `AccountReconnect` opens the sign-in
  in the browser, a dialog waits for `SignInWait` (Cancel, Copy link).
- Notifications (`gio::Notification`, the portal in the Flatpak) for
  `AuthRequired` (button "Sign In"), `UploadFailed` and `CacheFull`
  (button "Preferences"). The engine emits `AuthRequired` when the provider
  refuses the token (`operations/about` with `invalid_grant`…, checked with
  the quota); the account then shows an error and file commands answer
  `auth_required` (exit code 4). `UploadFailed` follows the count of
  errored files of `vfs/stats`.
- Cache settings (`<account>:cache-limit`, `<account>:cache-max-age`, the
  global `cache-max-age`) apply at once to mounted accounts by remounting
  the hidden rclone mount; the user's folder stays.
- Quit: `DaemonStop` with the "quit with pending uploads" setting; with
  "ask", a dialog offers Wait / Quit Now.
- Targets GTK 4.14 and libadwaita 1.5 (Ubuntu 24.04, CI); the GNOME 51
  runtime is newer.

## Offline pinning (v1.1)

Design: `docs/research/2026-11-01-milestone-12.md`.

- `nuage-fs` sends a pinned path to the pin store (`data/pins/<account>`)
  once its copy is there ([`PinView`] from the engine: pins, items deleted
  locally, saved listings). Temporary files saved next to a pinned file
  (`db.kdbx.Ab12Cd`) are pinned too, so saving by rename stays in the store.
- Offline (`MountedFs::set_offline`), pinned items work, the others are
  listed from the saved listings and fail with `EHOSTUNREACH` at once.
- Engine (`pins.rs`, `pindb.rs`): SQLite with the pins, the reference state
  of each item (remote size/date/hash, local size/date) and saved listings.
  At every `offline-check-interval` (30 s) it checks the connection
  (`operations/list` of the root), switches the folder offline or online,
  and syncs: one side changed → it wins; both → conflict copy, both kept.
  Downloads go through `data/pins-staging`, uploads wait 3 s of rest and the
  end of a pause.
- Without network at start-up, `nuage-fs` mounts alone and rclone's mount
  follows when the connection is back. `NUAGE_TEST_OFFLINE=<file>`
  simulates the loss of the network in tests.

## Flatpak package

`build-aux/flatpak/io.github.felixilgatto.Nuage.yml`, GNOME 51 runtime:

- `rclone`: the official archive, pinned by sha256 (x86_64, aarch64), and
  its license.
- `nuage`: an offline `cargo build --release` of `nuaged`, `nuage` and
  `nuage-gtk` from the vendored crates of `cargo-sources.json`
  (`scripts/cargo-sources.py`, from `Cargo.lock`; CI checks it is current);
  `nuage-fuse-relay` linked statically (it runs on the host); the
  `fusermount3` wrapper; the desktop entry, metainfo and icons of `data/`.
- Permissions: Wayland/X11, DRI, IPC, network, `home`, and the bus names of
  `org.freedesktop.Flatpak` (the wrapper only), the tray watcher and the
  Secret service. Never `--filesystem=host`.

`release.yml` (tag `v*`) rebuilds the bundle and the native packages and
publishes them with their checksums and the `CHANGELOG.md` section.

## Native packages

`scripts/build-native.sh` builds `nuaged` and `nuage` for
`x86_64-unknown-linux-musl` (static: they run on any x86-64 Linux, whatever
its glibc), fetches the pinned rclone (`scripts/fetch-rclone.sh`, sha256
checked) and produces in `target/dist/`:

- `nuage-<version>-x86_64-linux.tar.gz`: binaries, rclone,
  `build-aux/native/install.sh` (installs into `~/.local` without root) and
  the systemd user unit;
- `nuage_<version>_amd64.deb` (`cargo-deb`, metadata in
  `crates/daemon/Cargo.toml`): `/usr/bin/{nuaged,nuage}`,
  `/usr/lib/nuage/rclone`, `/usr/lib/systemd/user/nuaged.service`,
  depends on `fuse3`.

`nuaged` looks for rclone in `NUAGE_RCLONE`, `/app/bin/rclone` (Flatpak),
`../lib/nuage/rclone` next to its executable (both packages), then `PATH`.
The rc client speaks plain HTTP to 127.0.0.1: there is no TLS stack, hence
no system certificates and no C code in the build. The unit restarts the
daemon on failure every 10 s without limit.

## Tests

- Unit tests in each crate.
- `crates/fs/tests/fs_suite.rs`: `nuage-fs` over a plain folder: read,
  write, append, truncate, large files, seeks, directories with 1000
  entries, rename (with children, over a file, `RENAME_NOREPLACE`,
  `RENAME_EXCHANGE`, open files), deletion of open files, attributes,
  links, `statfs`, concurrent access, waiting for and timing out on an
  unavailable backing folder.
- `crates/fuse-relay/tests/relay.rs`: a real rclone mount through the relay.
- CI `native` job: the end-to-end tests against the static `nuaged`, then
  `scripts/smoke-native.sh` installs the `.deb` in clean Ubuntu 24.04 and
  Debian 12 containers, mounts a `local` account and reads and writes
  through it.
- `crates/cli/tests/e2e.rs`: real `nuaged` and `nuage` processes, a real
  rclone and FUSE, the `local` provider, an isolated home, plus WebDAV and
  S3 accounts against `rclone serve` (connection check, one bucket as the
  account root). Covers adding,
  renaming, mounting (both layers), unmounting, removing accounts, pause
  across restarts, recovery after `SIGKILL` of the daemon, the user's
  folder surviving `SIGKILL` of rclone, settings and error codes. CI installs the
  pinned rclone and `fuse3` and sets `NUAGE_E2E_REQUIRED=1`.
