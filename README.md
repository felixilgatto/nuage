![Nuage: your cloud files, simply on Linux.](images/social-preview.png)

[![CI](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml/badge.svg)](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Nuage puts your cloud storage in a regular folder on Linux.** Open it in
Files, work on your documents as usual: Nuage keeps them in sync.

> **Early development.** Google Drive works today. The desktop app runs from
> a source build; there is no Flatpak bundle yet.

## What it does

- Each account appears as a folder you choose, visible in Files.
- An icon in the top bar tells you whether everything is up to date.
- See how much space is used and free.
- Pause and resume uploads.
- Leave some folders out of sync.
- Works on servers too, from the command line.

Providers: **Google Drive** today; OneDrive and others later
([details](docs/PROVIDERS.md)).

## Install

**Desktop**: a Flatpak bundle will be attached to each release *(planned)*:

```sh
flatpak install --user nuage.flatpak
```

Until then, see [building from source](docs/BUILDING.md).

**Server**: a `.deb` and a tarball, see [docs/SERVER.md](docs/SERVER.md).

Requirement: `fuse3` (installed by default on Ubuntu).

## Get started

1. Open Nuage and pick a provider.
2. Sign in in your browser.
3. Choose the folder.

That's it. For your first Google account, Nuage first guides you through
creating your own Google access key (five minutes, free, done once; see
[why and how](docs/GOOGLE_OAUTH.md)).

Then Nuage lives in the top bar. Closing the window does not stop the sync.

## Command line

```sh
nuage status
nuage account add gdrive --client-id <id> --client-secret <secret>
nuage ls "Google Drive:Documents"
```

All commands: [docs/CLI.md](docs/CLI.md).

## Help

- [Providers](docs/PROVIDERS.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Nuage on a server](docs/SERVER.md)
- [Command line](docs/CLI.md)
- [Building and contributing](docs/BUILDING.md)
- [Adding a provider](docs/ADDING_A_PROVIDER.md)
- [Architecture](docs/ARCHITECTURE.md)

## Flatpak permissions

- `--talk-name=org.freedesktop.Flatpak`: used for one thing only, mounting
  and unmounting your folders with your system's `fusermount3`, through a
  small helper shipped with the app (`nuage-fuse-relay`). Nothing else runs
  outside the sandbox.
- `--filesystem=home`: lets you choose any folder in your home as a mount
  folder (it must have the same path inside and outside the sandbox).

## License

MIT, see [LICENSE](LICENSE). rclone, bundled in the packages, is also
MIT-licensed.
