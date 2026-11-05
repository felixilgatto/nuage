# Installing Nuage

## What it does

- Each account appears as a folder you choose, visible in Files.
- An icon in the top bar tells you whether everything is up to date.
- See how much space is used and free.
- Pause and resume uploads.
- Leave some folders out of sync.
- Keep files and folders available offline (e.g. a password database).
- Works on servers too, from the command line.

## Desktop (Flatpak)

For x86-64 and ARM64. You need Flatpak (on Ubuntu:
`sudo apt install flatpak`) and `fuse3` (installed by default on Ubuntu).
Open [nuage.flatpakref](https://felixilgatto.github.io/nuage/nuage.flatpakref)
with Software, or run:

```sh
flatpak install --user https://felixilgatto.github.io/nuage/nuage.flatpakref
```

Nuage then updates with `flatpak update` and in Software, like any other
app. It also says when a new version is available: click **Update**, then
**Restart** once it is installed. The first time, your desktop asks whether
Nuage may update itself. The GNOME runtime it needs comes from Flathub, added on the way if
needed. Each [release](https://github.com/felixilgatto/nuage/releases) also
has `.flatpak` bundles, which update the same way.

To build it yourself, see [BUILDING.md](BUILDING.md).

## Get started

1. Open Nuage and pick a provider.
2. Sign in: in your browser, or, for Nextcloud, WebDAV, S3 and Proton
   Drive, with an address and a password or access key.
3. Choose the folder.

For your first Google account, Nuage first guides you through creating
your own Google access key (five minutes, free, done once; see
[why and how](GOOGLE_OAUTH.md)). Then Nuage lives in the top bar. Closing
the window does not stop the sync.

## Server

`.deb` (Debian, Ubuntu), `.rpm` (Fedora, openSUSE) and a tarball, for
x86-64 and ARM64: see [SERVER.md](SERVER.md).

## Uninstall (desktop)

1. Quit Nuage (*Quit*, in its menu in the top bar or in the window's main
   menu), so your folders are unmounted. Avoid `flatpak kill`, which leaves
   a broken folder behind.
2. Remove the app and its data (settings, sign-ins, files kept offline):

   ```sh
   flatpak uninstall --user --delete-data io.github.felixilgatto.Nuage
   flatpak remote-delete --user nuage
   rm -f ~/.config/autostart/io.github.felixilgatto.Nuage.desktop
   ```

   The second line removes Nuage's repository (named `nuage-origin` instead
   if you installed a `.flatpak` bundle). Flatpak does not remove the
   start-at-login entry: the third line does. Your cloud folders stay,
   empty; delete them if you want.
3. Optionally, withdraw Nuage's access on the provider's side: Google
   <https://myaccount.google.com/connections>, Microsoft
   <https://account.live.com/consent/Manage>, Dropbox
   <https://www.dropbox.com/account/connected_apps>; or delete the app
   password or access key you created for Nuage.

## Flatpak permissions

- `--talk-name=org.freedesktop.Flatpak`: used for one thing only, mounting
  and unmounting your folders with your system's `fusermount3`, through a
  small helper shipped with the app (`nuage-fuse-relay`). Nothing else runs
  outside the sandbox.
- `--filesystem=home`: lets you choose any folder in your home as a mount
  folder (it must have the same path inside and outside the sandbox).
