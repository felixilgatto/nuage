# Nuage architecture

Nuage is an interface on top of rclone's remote control API (rc). rclone does
the mounting, caching, OAuth and transfers; Nuage does not reimplement a sync
engine. The one exception is `nuage-fs`, an overlay filesystem for exact
exclusions and, in v1.1, offline pinning.

> Status: skeleton (milestone 2). Interfaces exist; most engine operations
> return `not_implemented`.

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
- Mounts: `nuage-fs` at the user's folder over a hidden rclone mount;
  `MountPlan` keeps the two paths apart so the account configuration does
  not change when `nuage-fs` lands.
