# Nuage on a server (headless)

Nuage runs on a server without a screen, entirely from the command line,
with no interactive prompt. The packages contain static binaries and the
rclone version Nuage is tested with (installed apart from any other
rclone). Until the first release, download them from the `nuage-native`
artifact of a [CI run](https://github.com/felixilgatto/nuage/actions), or
build them with `scripts/build-native.sh`.

Requirements: Linux x86-64 with systemd, and `fuse3`.

## Install

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

## Connect Google Drive

You need your own OAuth client first: see
[GOOGLE_OAUTH.md](GOOGLE_OAUTH.md). Then pick one of two ways to sign in.

**With an SSH tunnel** (simplest): on the server, run

```sh
nuage account add gdrive --client-id <client ID> --client-secret <client secret> --no-browser
```

It prints a link to a page served by the server on port 53682. On your
computer, open a tunnel, then open the link in your browser:

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
    --token-file token.json --mount-point ~/Cloud/Drive
nuage status
```

To sign in again later, run `nuage account reconnect "Google Drive"` with
the same option (`--no-browser` or `--token-file`).

## Connect OneDrive

No client to create. **With an SSH tunnel**, on the server:

```sh
nuage account add onedrive --no-browser
```

then open the tunnel and the link as for Google Drive above. **With a token
made elsewhere**: on a computer with a browser and rclone, run
`rclone authorize onedrive`, save the JSON it prints as `token.json`, then on
the server:

```sh
nuage account add onedrive --token-file token.json --mount-point ~/Cloud/OneDrive
```

## Connect Dropbox or pCloud

Like OneDrive: `nuage account add dropbox --no-browser` (or `pcloud`), or a
token made elsewhere with `rclone authorize dropbox` (or `pcloud`) and
`--token-file token.json`.

## Connect Nextcloud, WebDAV or S3

No browser: the form's fields are `--param` options.

```sh
nuage account add nextcloud --param server=https://cloud.example.com \
    --param user=<user name> --param pass=<app password>
nuage account add webdav --param url=https://dav.example.com/files \
    --param user=<user name> --param pass=<password>
nuage account add s3 --param access_key_id=<key> --param secret_access_key=<secret> \
    --param endpoint=<endpoint, empty for Amazon S3> --param region=<region, if needed> \
    --param bucket=<bucket, or empty for all>
```

Passwords end up in your shell history: start the line with a space if your
shell skips those, or clear it afterwards.

## Logs and uninstall

- Logs: `journalctl --user -u nuaged`.
- Uninstall: `sudo apt remove nuage`, or `install.sh --uninstall`. Your
  settings stay in `~/.config/nuage`.

See [CLI.md](CLI.md) for every command.
