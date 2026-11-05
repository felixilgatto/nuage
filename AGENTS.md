# AGENTS.md

Instructions for coding agents working on Nuage. For people, start with
[README.md](README.md) and [docs/BUILDING.md](docs/BUILDING.md).

## Project overview

Nuage puts cloud storage (Google Drive, OneDrive, Nextcloud, WebDAV, S3,
Dropbox, pCloud, Proton Drive) in a regular folder on Linux. It is a Rust
workspace on top of rclone's remote control API: it does not implement its
own sync engine.

- `crates/daemon` (`nuaged`) runs the engine (`crates/engine`) and drives
  rclone through `crates/rclone-rc`.
- The clients talk to it over `crates/ipc`: the CLI (`crates/cli`, binary
  `nuage`), the GTK 4 / libadwaita app (`crates/gtk`, `crates/ui`) and the
  tray icon (`crates/tray`).
- `crates/providers`: one module and one Cargo feature per provider.
- `crates/fs`: the `nuage-fs` overlay (exclusions, offline pinning).
- `crates/fuse-relay`: runs `fusermount3` outside the Flatpak sandbox.

The specification, principles and milestones are in
[docs/brief.md](docs/brief.md). The design is in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Setup and build

- `cargo` may not be on `PATH`: it is in `~/.cargo/bin`.
- `cargo build` builds every crate except the GTK ones (`nuage-ui`,
  `nuage-gtk`). Those need the GTK 4 and libadwaita dev packages, or the
  GNOME SDK (`org.gnome.Sdk//51`); see "Desktop app" in
  [docs/BUILDING.md](docs/BUILDING.md). If they don't build in your shell,
  test with explicit `-p` lists.
- Proton Drive is outside the default features:
  `--features nuage-providers/protondrive`.
- Packages: `scripts/build-native.sh` (tarball, `.deb`, `.rpm`);
  Flatpak manifest in `build-aux/flatpak/`.

## Testing

Run these before you call a change done (CI runs the same):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
scripts/check-deps.sh
scripts/check-release.sh
cargo deny check
```

- The end-to-end tests (`crates/cli/tests/e2e.rs`) start real `nuaged`
  and `nuage` processes. They need FUSE and the **official** rclone
  binary: the rclone snap cannot mount. Point `NUAGE_RCLONE` at it.
  Without it they skip silently; `NUAGE_E2E_REQUIRED=1` makes that a
  failure.
- After any dependency change, run `scripts/cargo-sources.py` and commit
  `build-aux/flatpak/cargo-sources.json` (the Flatpak build is offline).
- Add or update tests for the code you change. A daemon restart in a test
  is followed by `wait_mounted`, never by a fixed sleep.

## Code style and rules

- **Everything is in English**: code, comments, commit messages, docs, UI
  and CLI strings. UI strings go through gettext.
- Messages shown to the user are for non-technical people: say what
  happened and what to do.
- Everything provider-specific stays in its module in `crates/providers`.
  The engine and interfaces adapt to a provider's capabilities, never to
  its name. Adding a provider touches no other crate; see
  [docs/ADDING_A_PROVIDER.md](docs/ADDING_A_PROVIDER.md).
- Everything the desktop app does must also be possible from the CLI.
- Keep the interface minimal: no new setting when a good default exists.
- Dependencies must be MIT-compatible (`cargo deny` enforces it).
- Flatpak permissions stay minimal; each one is justified in
  [docs/INSTALL.md](docs/INSTALL.md).

## Documentation

- User docs are in `docs/`, short and for non-technical readers. The
  README stays minimal; details go in `docs/`.
- When a provider changes, follow `.claude/skills/provider-docs/SKILL.md`.
- Add user-visible changes under `## [Unreleased]` in
  [CHANGELOG.md](CHANGELOG.md).
- Keep `docs/brief.md` in line with the current state.

## Commits and releases

- Commit and push only when asked. Commit messages: one imperative line
  describing the change ("Fix …", "Add …").
- Never push a `v*` tag or publish a release without the maintainer's
  explicit go. The procedure is in "Releases" in
  [docs/BUILDING.md](docs/BUILDING.md).

## Security

- Never commit secrets, tokens or OAuth client credentials. The Flatpak
  signing key exists only as a repository secret: never print, regenerate
  or replace it (changing it breaks updates for every installed copy).
- OAuth scopes stay limited to files.
