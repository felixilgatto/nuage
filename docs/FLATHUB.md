# Publishing on Flathub

Flathub builds Nuage from a published release, reviews the manifest, then
hosts it. Submitting is done by the maintainer, from their GitHub account.

## Before submitting

- A release tag `v<version>` is pushed (see [BUILDING.md](BUILDING.md),
  "Releases"), with its `<release>` entry in
  `data/io.github.felixilgatto.Nuage.metainfo.xml`.
- The screenshots of the metainfo are reachable on the `main` branch.
- CI is green, including `scripts/flatpak-lint.sh`.

## The permission to explain

Nuage needs `--talk-name=org.freedesktop.Flatpak` for one thing: mounting
folders. Neither rclone nor `nuage-fs` can mount from inside the sandbox
(`fusermount3` is setuid and absent there), so the `fusermount3` wrapper
runs the host's `fusermount3` through `flatpak-spawn --host`, via
`nuage-fuse-relay` (see [ARCHITECTURE.md](ARCHITECTURE.md), "Flatpak:
mounting from the sandbox"). Nothing else uses it.

Flathub treats this permission as an escape from the sandbox and grants it
by exception only (`finish-args-flatpak-spawn-access` in the linter). The
submission must explain the reason above and link the wrapper
(`build-aux/flatpak/fusermount-wrapper.sh`) and the relay
(`crates/fuse-relay`). The review may still refuse it; in that case Nuage
stays available as a bundle on the GitHub releases. `--filesystem=home`
(mount folders chosen freely in the home folder) also needs a sentence.

## Submitting

1. Generate the manifest of the release:

   ```sh
   scripts/flathub-manifest.py v0.1.0      # writes target/flathub/
   ```

   It is the manifest of `build-aux/flatpak/` with the release's git tag
   and commit as source, and `cargo-sources.json`.
2. Check it builds from the published sources:

   ```sh
   flatpak run org.flatpak.Builder --force-clean --repo=target/flathub-repo \
     --state-dir=target/flatpak-builder target/flathub-build \
     target/flathub/io.github.felixilgatto.Nuage.yml
   LINT="flatpak run --command=flatpak-builder-lint org.flatpak.Builder" \
     scripts/flatpak-lint.sh target/flathub-repo
   ```

3. Follow <https://docs.flathub.org/docs/for-app-authors/submission>: fork
   `flathub/flathub`, branch from `new-pr`, add the two files of
   `target/flathub/`, and open a pull request against `new-pr` titled
   "Add io.github.felixilgatto.Nuage", with the explanation of the
   permissions above.
4. The `io.github.felixilgatto` ID is verified by Flathub through the
   GitHub account `felixilgatto`.

Once accepted, Flathub creates `flathub/io.github.felixilgatto.Nuage`; each
new release updates the tag and commit there (the same script).
