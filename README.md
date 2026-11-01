# Nuage

[![CI](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml/badge.svg)](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Your cloud storage as a regular folder on Linux: a GNOME app for the desktop, a command-line tool for servers.**

> **Status: early development.** The daemon and the command-line tool mount
> Google Drive and local folders, pause and resume uploads and report the
> sync state. The desktop app (setup, accounts, tray icon) works from a
> source build; there is no Flatpak bundle yet.
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
| Google Drive ("My Drive") | in development | yes | yes | yes |
| Local folder (tests) | in development | yes | no | no |
| OneDrive, others | planned | | | |

## Install on the desktop *(planned)*

A Flatpak bundle will be attached to each release:

```sh
flatpak install --user nuage.flatpak
```

Requirement: `fuse3` (installed by default on Ubuntu).

## Install on a server (headless)

Nuage runs on a server without a screen, entirely from the command line,
with no interactive prompt. The packages contain static binaries and the
rclone version Nuage is tested with (installed apart from any other
rclone). Until the first release, download them from the `nuage-native`
artifact of a [CI run](https://github.com/felixilgatto/nuage/actions), or
build them with `scripts/build-native.sh`.

Requirements: Linux x86-64 with systemd, and `fuse3`.

**With the `.deb`** (Debian, Ubuntu; installs `fuse3` too):

```sh
sudo apt install ./nuage_<version>_amd64.deb
```

**Or with the tarball**, without root (installs into `~/.local`):

```sh
tar xf nuage-<version>-x86_64-linux.tar.gz
./nuage-<version>-x86_64-linux/install.sh
```

Then start the daemon, and keep it running when you are logged out:

```sh
systemctl --user enable --now nuaged
loginctl enable-linger "$USER"     # also mounts your folders at boot
```

Connect Google Drive, with your own OAuth client (see
[Create your own Google OAuth client](#create-your-own-google-oauth-client)),
either through an SSH tunnel or with a token made on another computer (see
[On a server without a browser](#on-a-server-without-a-browser)):

```sh
nuage account add gdrive --client-id <id> --client-secret <secret> \
    --token-file token.json --mount-point ~/Cloud/Drive
nuage status
```

Logs: `journalctl --user -u nuaged`. Uninstall: `sudo apt remove nuage`, or
`install.sh --uninstall` (your settings stay in `~/.config/nuage`).

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
scripts/build-native.sh   # static tarball and .deb in target/dist/ (needs the
                          # x86_64-unknown-linux-musl target and cargo-deb)
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

## First launch

Pick a provider, sign in in your browser, choose the folder: three screens.
For your first Google account, a screen in between guides you through
creating your own OAuth client (see below). Then Nuage lives in the system
tray: on Ubuntu, make sure the "Ubuntu AppIndicators" extension is enabled
(Extensions app); without it, Nuage keeps its window open instead.

## Create your own Google OAuth client

Google Drive needs an OAuth client of your own: rclone's shared client stops
working in 2026, and a client shared by all Nuage users would need a costly
Google audit. Creating yours takes about five minutes and is free. You do it
once; your other Google accounts reuse it.

1. **Create a project**: open <https://console.cloud.google.com/projectcreate>,
   name it (e.g. "Nuage"), click **Create**, and make sure it is selected at
   the top of the page.
2. **Enable the Drive API**: open
   <https://console.cloud.google.com/apis/library/drive.googleapis.com> and
   click **Enable**.
3. **Describe the app**: open <https://console.cloud.google.com/auth/branding>,
   click **Get started**, give an app name (e.g. "Nuage for me") and your
   e-mail address, choose **External** as audience, and finish.
4. **Publish it**: open <https://console.cloud.google.com/auth/audience> and
   click **Publish app**, then confirm. This matters: while the app is in
   *Testing*, Google makes you sign in again every 7 days. Google does not
   review a personal app; it only warns, when you sign in, that the app is
   not verified (click **Advanced**, then **Go to …**: it is your own app).
5. **Create the client**: open <https://console.cloud.google.com/auth/clients>,
   click **Create client**, choose the type **Desktop app**, and create it.
   Copy the **Client ID** and the **Client secret**.

Nuage asks Google for access to your Drive files only (`drive` scope), no
other Google service.

## Connect Google Drive

On a computer with a web browser:

```sh
nuage account add gdrive --client-id <client ID> --client-secret <client secret>
```

Your browser opens; sign in and allow access. The account is mounted at
`~/Cloud/Google Drive` (choose another folder with `--mount-point`). Google
Docs, Sheets and Slides appear as `.link.html` shortcuts that open in the
browser.

### On a server without a browser

**With an SSH tunnel** (simplest): on the server, run the same command with
`--no-browser`. It prints a link to a page served by the server on port
53682. On your computer, open a tunnel, then open the link in your browser:

```sh
ssh -L 53682:localhost:53682 <server>     # on your computer, keep it open
```

**With a token made elsewhere** (fully scriptable): on any computer with a
browser and [rclone](https://rclone.org/install/), run

```sh
rclone authorize drive <client ID> <client secret>
```

sign in, and copy the JSON it prints (between the arrows) into a file
`token.json`. Then on the server:

```sh
nuage account add gdrive --client-id <client ID> --client-secret <client secret> \
    --token-file token.json
```

If Google later asks you to sign in again (for example after you changed
your password), run `nuage account reconnect "Google Drive"`, with the same
options (`--no-browser` or `--token-file`).

### Where your access is stored

The sign-in tokens are in rclone's configuration. On a desktop, it is
encrypted with a password kept in your keyring. On a server without a
keyring, it is not encrypted but readable by your user only; `nuage status`
says so.

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
- **A Google Doc opens in the wrong application** (e.g. Thunderbird): Docs,
  Sheets and Slides are `.link.html` shortcuts, opened by your default
  application for web pages. Make your browser that application, e.g.
  `xdg-mime default firefox.desktop text/html` (or in Settings → Apps →
  Default apps).
- **Google asks you to sign in every week**: your OAuth app is still in
  *Testing*; publish it (step 4 of the guide above), then run
  `nuage account reconnect "Google Drive"`.
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

Working so far: `status [--watch]`, `account add | list | rename | reconnect
| remove`, `mount`, `unmount`, `pause`, `resume`, `refresh`, `exclude add |
remove | list`, `config get | set` and `daemon status | stop`. File commands
(`ls`, `cp`…) and pins answer "not implemented yet" (exit code 1).

### Exclusions

Items you exclude are neither downloaded nor uploaded, and disappear from
the folder; they stay intact in the cloud.

```sh
nuage exclude add "Google Drive" Archives            # a folder of the account
nuage exclude add "Google Drive" --glob 'node_modules/'   # a pattern, anywhere
nuage exclude list
nuage exclude remove "Google Drive" Archives
```

Temporary and lock files of office suites are excluded by default (`~$*`,
`.~lock.*#`, `*.tmp`, `.goutputstream-*`); turn that off with
`nuage config set "<account>:default-exclusions" off`. A file you create in
the folder whose name is excluded stays on this computer only, at its place
in the folder. Changing an exclusion briefly remounts the account; if
uploads are pending, add `--yes`. Run
`nuage --help` for the full list.

Settings (`nuage config get`, `nuage config set <key> <value>`):

| Key | Value |
|---|---|
| `cache-limit` | cache limit of new accounts, e.g. `20G` |
| `<account>:cache-limit` | cache limit of one account |
| `cache-max-age` | how long unused files stay in the cache, e.g. `30d` |
| `<account>:default-exclusions` | `on` or `off`: exclude temporary and lock files |
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
