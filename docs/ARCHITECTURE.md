# Nuage architecture

Nuage is an interface on top of rclone's remote control API (rc). rclone does
the mounting, caching, OAuth and transfers; Nuage does not reimplement a sync
engine. The one exception is `nuage-fs`, an overlay filesystem for exact
exclusions and, in v1.1, offline pinning.

> Status: milestone 3. The daemon supervises rclone, mounts accounts,
> pauses uploads and reports the status; the CLI drives all of it. File
> operations, exclusions, Google sign-in and pinning still return
> `not_implemented`.

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
| `crates/fs` | `nuage-fs` | overlay FUSE filesystem (placeholder) |
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
- Mounts: `nuage-fs` at the user's folder over a hidden rclone mount;
  `MountPlan` keeps the two paths apart so the account configuration does
  not change when `nuage-fs` lands. Until milestone 4, rclone mounts
  directly at the user's folder.

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
- `crates/cli/tests/e2e.rs`: real `nuaged` and `nuage` processes, a real
  rclone and FUSE, the `local` provider, an isolated home. Covers adding,
  renaming, mounting, unmounting, removing accounts, pause across restarts,
  recovery after `SIGKILL`, settings and error codes. CI installs the
  pinned rclone and `fuse3` and sets `NUAGE_E2E_REQUIRED=1`.
