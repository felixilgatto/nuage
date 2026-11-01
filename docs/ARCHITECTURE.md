# Nuage architecture

Nuage is an interface on top of rclone's remote control API (rc). rclone does
the mounting, caching, OAuth and transfers; Nuage does not reimplement a sync
engine. The one exception is `nuage-fs`, an overlay filesystem for exact
exclusions and, in v1.1, offline pinning.

> Status: milestone 4. The daemon supervises rclone, mounts each account
> in two layers (`nuage-fs` over a hidden rclone mount), pauses uploads and
> reports the status; the CLI drives all of it. File operations,
> exclusions, Google sign-in and pinning still return `not_implemented`.

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
policy. Real capabilities are the static ones merged with rclone's
`operations/fsinfo` features. Interfaces hide actions according to
capabilities, never according to the provider name. Adding a provider means
adding a module and a Cargo feature in `providers`, without touching
`engine`, `ui`, `tray` or `cli`.

## IPC protocol

- Socket: `$XDG_RUNTIME_DIR/nuage/nuaged.sock` (mode `0600`, directory
  `0700`). Inside the Flatpak: `$XDG_RUNTIME_DIR/app/io.github.felixilgatto.Nuage/nuaged.sock`,
  which the host can also reach; the CLI tries both. `NUAGE_SOCKET` overrides.
- Framing: one JSON object per line.
- Handshake: the client sends `{"type":"hello","protocol_version":1,"client":"…"}`;
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
- Orphans: a `SIGKILL`ed daemon leaves `fuse.nuage` and `fuse.rclone`
  mounts; the next start detaches them, except user folders it still serves.

### Flatpak: mounting from the sandbox

The sandbox has neither `fusermount3` nor the right to mount.
`/app/bin/fusermount3` is `build-aux/flatpak/fusermount-wrapper.sh`:

1. libfuse (rclone) or `nuage-fs` run it with a socket in `$_FUSE_COMMFD`.
2. It runs `nuage-fuse-relay` on the host with `flatpak-spawn --host
   --forward-fd`, from the application's host path (`app-path` in
   `/.flatpak-info`).
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
  mounts it, and rolls everything back if a step fails.
- The `local` provider is an rclone `alias` of a folder, so every account's
  root is `<remote>:`. Two `local` accounts over the same folder share one
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
`StatusReport.config_encrypted` is `false`. An encrypted file without its
keyring stops the daemon with a message. The password is never logged
(`RcdSupervisor`'s `Debug` hides it).

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
| last mount failed | `error` (with the reason) |
| not mounted | `stopped` |
| upload errors, or cache disk full | `error` |
| paused | `paused { queued }` |
| uploads in progress or queued | `syncing { uploads, queued }` |
| otherwise | `up_to_date` |

### Files

| Content | Native | Flatpak (`~/.var/app/<id>/`) |
|---|---|---|
| `config.toml` (accounts, pause, settings), mode `0600` | `~/.config/nuage/` | `config/nuage/` |
| `rclone.conf` | `~/.config/nuage/rclone.conf` | `config/rclone/rclone.conf` |
| VFS cache | `~/.cache/nuage/rclone/` | `cache/rclone/` |
| `rcd.pid`, hidden mounts | `~/.local/share/nuage/` | `data/` |

## Tests

- Unit tests in each crate.
- `crates/fs/tests/fs_suite.rs`: `nuage-fs` over a plain folder: read,
  write, append, truncate, large files, seeks, directories with 1000
  entries, rename (with children, over a file, `RENAME_NOREPLACE`,
  `RENAME_EXCHANGE`, open files), deletion of open files, attributes,
  links, `statfs`, concurrent access, waiting for and timing out on an
  unavailable backing folder.
- `crates/fuse-relay/tests/relay.rs`: a real rclone mount through the relay.
- `crates/cli/tests/e2e.rs`: real `nuaged` and `nuage` processes, a real
  rclone and FUSE, the `local` provider, an isolated home. Covers adding,
  renaming, mounting (both layers), unmounting, removing accounts, pause
  across restarts, recovery after `SIGKILL` of the daemon, the user's
  folder surviving `SIGKILL` of rclone, settings and error codes. CI installs the
  pinned rclone and `fuse3` and sets `NUAGE_E2E_REQUIRED=1`.
