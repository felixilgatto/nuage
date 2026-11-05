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
                          # x86_64-unknown-linux-musl target, cargo-deb and
                          # musl-tools: SQLite is compiled in with musl-gcc)
export NUAGE_RCLONE=/path/to/rclone
cargo run -p nuaged &  # start the daemon (socket in $XDG_RUNTIME_DIR/nuage/)
mkdir -p /tmp/files && echo hello > /tmp/files/hello.txt
cargo run -p nuage -- account add local --param root=/tmp/files
cargo run -p nuage -- status
```

Proton Drive (preview) is left out of the default build; add it with
`--features nuage-providers/protondrive` (with `--workspace` for the desktop
app too).

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
`org.flatpak.Builder` Flatpak; with the latter, add `--disable-rofiles-fuse`
if the build fails in `rofiles-fuse`, and keep the state and build
directories under your home):

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

CI builds the bundle (artifact `nuage-x86_64.flatpak`) and runs
`scripts/flatpak-lint.sh` (`flatpak-builder-lint` on the manifest, the
metainfo and the repository). It accepts only these findings for now:
access to `flatpak-spawn` (the `fusermount3` wrapper, see
[ARCHITECTURE.md](ARCHITECTURE.md)), access to `home` (mount folders are
chosen freely) and screenshots not mirrored in the repository (Flathub does
it at its own build). Screenshots are in `data/screenshots/`, taken with
`scripts/demo.sh` (sample files, light style, no personal data). Locally:

```sh
LINT="flatpak run --command=flatpak-builder-lint org.flatpak.Builder" \
  scripts/flatpak-lint.sh target/flatpak-repo
```

## Releases

A release publishes, for x86_64 and aarch64: the `.deb`, the `.rpm` and the
tarball (static binaries), the `.flatpak` bundles, their checksums and
their build provenance in a GitHub release; and the signed Flatpak
repository on GitHub Pages (<https://felixilgatto.github.io/nuage/>), from
which installed copies update.

1. `scripts/prepare-release.sh <version>`: sets the version in
   `Cargo.toml`, turns the changelog's `[Unreleased]` section into
   `[<version>] - <today>`, and adds the `<release>` of the metainfo. Review
   the diff and commit it. (Run it again on the day of the release to
   update the date.)
2. Test everything without publishing: Actions → **Release** → **Run
   workflow** on the branch. It builds every package on both
   architectures, installs them in clean Ubuntu, Debian and Fedora
   containers, builds the Flatpak repository signed with a throw-away key,
   and installs, updates and updates a bundle from it as a new user. The
   files are in the run's artifacts.
3. Push the tag: `git tag v<version> && git push origin v<version>`. The
   same workflow then publishes.

The homepage (<https://felixilgatto.github.io/nuage/>) is `website/index.html`;
`scripts/site-page.sh` puts it next to the Flatpak repository. A release
publishes it. To publish a change to it without a release: Actions →
**Site** → **Run workflow** on `main`, with *publish* ticked (without it, the
run only builds the site). It reuses the latest release's repository
unchanged (`nuage-site.tar`, attached to the release), so installed copies
see no update. To look at it locally:
`scripts/site-page.sh`, then open `target/site/index.html`.

`scripts/check-release.sh` (run by CI) checks that `Cargo.toml`, the
metainfo and the changelog agree; the release also checks the tag.

**Once, before the first release** (done on 2026-11-05: the key's
fingerprint is `9833 536F 5965 1DC9 E942  1FCF DFD2 049E 198F 83FB`):

- The Flatpak repository's signing key: a GPG key without passphrase, kept
  only as the `FLATPAK_SIGNING_KEY` secret of the repository (and a backup
  in a safe place). Changing it later breaks updates for everyone who
  installed Nuage, so do not lose it.

  ```sh
  export GNUPGHOME=$(mktemp -d)
  gpg --batch --passphrase '' --quick-gen-key "Nuage Flatpak repository" rsa4096 sign never
  gpg --armor --export-secret-keys | gh secret set FLATPAK_SIGNING_KEY
  gpg --armor --export-secret-keys > nuage-flatpak-signing-key.asc   # the backup
  rm -rf "$GNUPGHOME"
  ```

- GitHub Pages published by GitHub Actions, from `v*` tags too:

  ```sh
  gh api -X POST repos/felixilgatto/nuage/pages -f build_type=workflow
  gh api -X POST repos/felixilgatto/nuage/environments/github-pages/deployment-branch-policies \
      -f name='v*' -f type=tag
  ```

## Contributing

Before sending a change, run:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
NUAGE_E2E_REQUIRED=1 cargo test
scripts/check-deps.sh
cargo deny check
```

Publishing on Flathub: see [FLATHUB.md](FLATHUB.md).
