![Nuage: your cloud files, simply on Linux.](images/social-preview.png)

[![CI](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml/badge.svg)](https://github.com/felixilgatto/nuage/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Nuage puts your cloud storage in a regular folder on Linux.** Open it in
Files, work on your documents as usual: Nuage keeps them in sync.

Google Drive, OneDrive, Nextcloud, WebDAV and S3; Dropbox, pCloud and
Proton Drive in preview ([details](docs/PROVIDERS.md)).

## Install

```sh
flatpak install --user https://felixilgatto.github.io/nuage/nuage.flatpakref
```

Or open [nuage.flatpakref](https://felixilgatto.github.io/nuage/nuage.flatpakref)
with Software. Servers, `.deb`, `.rpm`, uninstalling: [docs/INSTALL.md](docs/INSTALL.md).

## Get started

Open Nuage, pick a provider, sign in, choose the folder. Nuage then lives
in the top bar ([more](docs/INSTALL.md#get-started)).

## Documentation

- [Installing, getting started, uninstalling](docs/INSTALL.md)
- [Providers](docs/PROVIDERS.md)
- [Troubleshooting](docs/TROUBLESHOOTING.md)
- [Command line](docs/CLI.md) and [servers](docs/SERVER.md)
- [Building and contributing](docs/BUILDING.md)

## License

MIT, see [LICENSE](LICENSE). rclone, bundled in the packages, is also
MIT-licensed.
