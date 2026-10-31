# Milestone 1 proof of concept: FUSE from the Flatpak sandbox

Throwaway Flatpak (`io.github.felixilgatto.NuagePoc`) whose command is a pinned rclone, plus the
`fusermount3` wrapper that relays mounts to the host. See `docs/research/2026-10-30-milestone-1.md`.

## Build

```sh
flatpak run org.flatpak.Builder --user --install --force-clean --disable-rofiles-fuse \
    build-dir io.github.felixilgatto.NuagePoc.yml
```

## Test with a local directory

```sh
mkdir -p ~/.cache/nuage-poc-src ~/Cloud/nuage-poc
echo hello > ~/.cache/nuage-poc-src/hello.txt
flatpak run io.github.felixilgatto.NuagePoc mount ":local:$HOME/.cache/nuage-poc-src" ~/Cloud/nuage-poc \
    --vfs-cache-mode full --daemon
cat ~/Cloud/nuage-poc/hello.txt
flatpak run --command=fusermount3 io.github.felixilgatto.NuagePoc -u ~/Cloud/nuage-poc
```

## Test with Google Drive

Needs your own OAuth client ("Desktop app" credentials from a GCP project with the Drive API enabled and the
consent screen published in Production).

```sh
flatpak run io.github.felixilgatto.NuagePoc config create gdrive drive \
    client_id=<id> client_secret=<secret> scope=drive
flatpak run io.github.felixilgatto.NuagePoc mount gdrive: ~/Cloud/nuage-poc-drive --vfs-cache-mode full --daemon
```

Then open `~/Cloud/nuage-poc-drive` in Nautilus.
