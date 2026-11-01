# Nuage

[![CI](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml/badge.svg)](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Your cloud storage as a regular folder on Linux: a GNOME app for the desktop, a command-line tool for servers.**

> **Status: early development.** The daemon and the command-line tool can
> mount a local folder (the test provider), pause and resume uploads and
> report the sync state; Google Drive and the desktop app are not ready yet.
> Sections marked *(planned)* describe the intended behaviour.

<!-- TODO: screenshot -->

## Features *(planned)*

- Each cloud account mounted in a folder you choose, visible in Files (Nautilus).
- A tray icon showing whether everything is up to date, syncing, paused or has a problem.
- Used and free space for each account.
- One file browser that looks the same for every provider.
- A limit on the local cache, per account (20 GB by default).
- Pause and resume uploads.
- Exclude folders or patterns (`*.tmp`, `node_modules/`…) from sync.
- Full command-line use on headless servers, over SSH, scriptable with `--json`.
- Offline mode: keep chosen files available without a network (v1.1).

## Providers

| Provider | Status | Quota | Share link | Trash |
|---|---|---|---|---|
| Google Drive ("My Drive") | planned for v1 | yes | yes | yes |
| Local folder (tests) | in development | yes | no | no |
| OneDrive, others | planned | | | |

## Install on the desktop *(planned)*

A Flatpak bundle will be attached to each release:

```sh
flatpak install --user nuage.flatpak
```

Requirement: `fuse3` (installed by default on Ubuntu).

## Install on a server (headless) *(planned)*

A `.deb` package and a tarball will be attached to each release:

```sh
sudo apt install ./nuage_<version>_amd64.deb      # or: tar xf … && ./install.sh
systemctl --user enable --now nuaged
loginctl enable-linger "$USER"                     # keep running without a session
nuage account add gdrive --token-file token.json --mount-point ~/Cloud/Drive
nuage status
```

## Building from source

Nuage is a Cargo workspace. The daemon (`nuaged`) and the command-line tool
(`nuage`) have no GTK dependency; the desktop app (`nuage-gtk`) needs GTK 4
and libadwaita.

### Daemon and CLI (any machine)

The daemon runs rclone, which needs FUSE (`fuse3`) to mount. The rclone
**snap cannot mount**: use the official binary from
<https://rclone.org/install/> and point `NUAGE_RCLONE` at it if it is not
first in your `PATH`.

```sh
cargo build            # default members: everything except the GTK crates
cargo test             # end-to-end tests are skipped without rclone and FUSE
export NUAGE_RCLONE=/path/to/rclone
cargo run -p nuaged &  # start the daemon (socket in $XDG_RUNTIME_DIR/nuage/)
mkdir -p /tmp/files && echo hello > /tmp/files/hello.txt
cargo run -p nuage -- account add local --param root=/tmp/files
cargo run -p nuage -- status
```

The end-to-end tests (`crates/cli/tests/e2e.rs`) start real `nuaged` and
`nuage` processes with a temporary home. They skip themselves when rclone or
FUSE is missing; `NUAGE_E2E_REQUIRED=1` (set in CI) makes that a failure.

### Desktop app, with system packages

On Ubuntu 24.04 or later:

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config
cargo build --workspace
cargo run -p nuage-gtk
```

### Desktop app, inside the GNOME SDK (no system packages needed)

Install the SDK and the Rust extension once, as a user:

```sh
flatpak install --user flathub org.gnome.Sdk//51 org.freedesktop.Sdk.Extension.rust-stable//26.08
```

Then build in the SDK. A separate `CARGO_HOME` and target directory keep it
apart from the host toolchain:

```sh
flatpak run --command=bash --filesystem="$PWD" --share=network \
  --env=PATH=/usr/lib/sdk/rust-stable/bin:/usr/bin:/bin \
  --env=CARGO_HOME="$PWD/.flatpak-cargo" \
  --env=CARGO_TARGET_DIR="$PWD/target/flatpak-sdk" \
  org.gnome.Sdk//51 -c "cd '$PWD' && cargo build --workspace"
```

And run the app in the SDK:

```sh
flatpak run --command="$PWD/target/flatpak-sdk/debug/nuage-gtk" \
  --filesystem="$PWD" --socket=wayland --socket=fallback-x11 --device=dri \
  --own-name=io.github.felixilgatto.Nuage \
  --talk-name=org.kde.StatusNotifierWatcher org.gnome.Sdk//51
```

The Flatpak manifest (`flatpak-builder`) comes later.

## First launch *(planned)*

Pick a provider, sign in, choose the folder: three screens. Google Drive
first asks you to create your own OAuth client (see below).

## Create your own Google OAuth client *(planned)*

A step-by-step guide (Google Cloud project, Drive API, consent screen in
*Production*, "Desktop app" credentials) will be written here.

## Flatpak permissions *(planned)*

Each permission and its reason will be listed here, in particular
`--talk-name=org.freedesktop.Flatpak` and `--filesystem=home`.

- `--talk-name=org.freedesktop.Flatpak` lets the app run programs on your
  computer outside the sandbox. Nuage uses it for one thing only: mounting
  and unmounting your folders with your system's `fusermount3`, through a
  small helper shipped with the app (`nuage-fuse-relay`). Nothing else runs
  outside the sandbox.
- `--filesystem=home` lets you choose any folder in your home as a mount
  folder: the folder must have the same path inside and outside the
  sandbox.

## Offline mode *(planned, v1.1)*

## Troubleshooting

- **`nuage` says the daemon is unreachable (exit code 3)**: start it with
  `systemctl --user start nuaged` (or `nuaged &` during development).
- **"mounting is not supported when running from snap"**: the rclone snap
  cannot mount folders. Install rclone from <https://rclone.org/install/>
  and set `NUAGE_RCLONE` to its path.
- **The daemon log**: `journalctl --user -u nuaged`, or the daemon's standard
  error during development (`NUAGE_LOG=debug` for more detail).

## Command line

Every command accepts `--json`. Exit codes:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error |
| 2 | incorrect usage |
| 3 | daemon unreachable |
| 4 | authentication required |

Working so far: `status [--watch]`, `account add | list | rename | remove`,
`mount`, `unmount`, `pause`, `resume`, `refresh`, `config get | set` and
`daemon status | stop`. File commands (`ls`, `cp`…), exclusions, pins and
Google sign-in answer "not implemented yet" (exit code 1). Run
`nuage --help` for the full list.

Settings (`nuage config get`, `nuage config set <key> <value>`):

| Key | Value |
|---|---|
| `cache-limit` | cache limit of new accounts, e.g. `20G` |
| `<account>:cache-limit` | cache limit of one account |
| `cache-max-age` | how long unused files stay in the cache, e.g. `30d` |
| `quit-with-pending` | what `nuage daemon stop` does while uploads are pending: `finish-in-background` (default), `ask` or `quit-now` |

Cache settings apply the next time the account is mounted.

## Architecture

```
nuaged  (daemon: engine + rclone rcd)
   ▲  Unix socket, newline-delimited JSON (Command / Event)
   ├─ nuage-gtk   desktop app: window + tray icon
   └─ nuage       CLI (humans, scripts, agents)
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Contributing

Before sending a change, run:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
NUAGE_E2E_REQUIRED=1 cargo test
scripts/check-deps.sh
cargo deny check
```

## License

MIT, see [LICENSE](LICENSE). rclone, bundled in the packages, is also
MIT-licensed.
