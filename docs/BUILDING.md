# Building from source

Nuage is a Cargo workspace. The daemon (`nuaged`) and the command-line tool
(`nuage`) have no GTK dependency; the desktop app (`nuage-gtk`) needs GTK 4
and libadwaita. See [ARCHITECTURE.md](ARCHITECTURE.md) for how the pieces
fit together.

## Daemon and CLI (any machine)

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

## Desktop app, with system packages

On Ubuntu 24.04 or later:

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev pkg-config
cargo build --workspace
cargo run -p nuage-gtk
```

A source build finds its icons in `data/icons` by itself.

## Desktop app, inside the GNOME SDK (no system packages needed)

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

## Flatpak

The manifest is `build-aux/flatpak/io.github.felixilgatto.Nuage.yml`. The
build is offline: crates come from `build-aux/flatpak/cargo-sources.json`,
generated from `Cargo.lock`. Regenerate it after any dependency change (CI
fails otherwise):

```sh
scripts/cargo-sources.py
```

Build, install for your user and run (needs `flatpak-builder`, or the
`org.flatpak.Builder` Flatpak):

```sh
flatpak run org.flatpak.Builder --user --install --force-clean \
  --state-dir=target/flatpak-builder target/flatpak-build \
  build-aux/flatpak/io.github.felixilgatto.Nuage.yml
flatpak run io.github.felixilgatto.Nuage
```

Or make a bundle to install elsewhere:

```sh
flatpak run org.flatpak.Builder --force-clean --repo=target/flatpak-repo \
  --state-dir=target/flatpak-builder target/flatpak-build \
  build-aux/flatpak/io.github.felixilgatto.Nuage.yml
flatpak build-bundle target/flatpak-repo nuage.flatpak io.github.felixilgatto.Nuage
```

CI builds the bundle (artifact `nuage.flatpak`) and runs
`scripts/flatpak-lint.sh` (`flatpak-builder-lint` on the manifest, the
metainfo and the repository). It accepts only these findings for now:
access to `flatpak-spawn` (the `fusermount3` wrapper, see
[ARCHITECTURE.md](ARCHITECTURE.md)), access to `home` (mount folders are
chosen freely) and the missing screenshots. Locally:

```sh
LINT="flatpak run --command=flatpak-builder-lint org.flatpak.Builder" \
  scripts/flatpak-lint.sh target/flatpak-repo
```

## Releases

Push a tag `v<version>` (after a `## [<version>]` section in
`CHANGELOG.md`): `release.yml` builds the Flatpak bundle, the tarball and
the `.deb`, and publishes them in a GitHub release with their checksums
and the changelog section.

## Contributing

Before sending a change, run:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
NUAGE_E2E_REQUIRED=1 cargo test
scripts/check-deps.sh
cargo deny check
```
