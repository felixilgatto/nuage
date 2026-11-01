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

## Milestone 4 check: nuaged in the sandbox

`io.github.felixilgatto.NuagePoc.m4.yml` packages `nuaged`, `nuage`, the production wrapper
(`build-aux/flatpak/fusermount-wrapper.sh`) and a static `nuage-fuse-relay`. Stage the binaries, then build:

```sh
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release -p nuage-fuse-relay --target x86_64-unknown-linux-gnu
# nuaged and nuage in the GNOME SDK (see the main README), into target/flatpak-sdk
mkdir -p m4-stage && cp ../../build-aux/flatpak/fusermount-wrapper.sh \
    ../../target/x86_64-unknown-linux-gnu/release/nuage-fuse-relay \
    ../../target/flatpak-sdk/release/{nuaged,nuage} m4-stage/
flatpak run org.flatpak.Builder --user --install --force-clean --disable-rofiles-fuse \
    build-dir io.github.felixilgatto.NuagePoc.m4.yml
```

Run the daemon in the sandbox and drive it from the host:

```sh
export NUAGE_SOCKET=/run/user/$(id -u)/app/io.github.felixilgatto.NuagePoc/nuaged.sock
flatpak run --env=NUAGE_SOCKET=$NUAGE_SOCKET io.github.felixilgatto.NuagePoc &
mkdir -p ~/.cache/nuage-m4/src && echo hello > ~/.cache/nuage-m4/src/hello.txt
nuage account add local --name M4 --mount-point ~/.cache/nuage-m4/mnt --param root=$HOME/.cache/nuage-m4/src
cat ~/.cache/nuage-m4/mnt/hello.txt
nuage account remove M4 --yes && nuage daemon stop --now
```
