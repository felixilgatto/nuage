---
name: provider-docs
description: Bring every piece of Nuage documentation that describes storage providers (Google Drive, local, OneDrive…) back in line with the code in crates/providers. Use after adding, removing or changing a provider (capabilities, sign-in, client guide, Cargo features), when the user asks to update or check the provider documentation, or before a release.
---

# Keep the provider documentation up to date

The code is the source of truth; the documentation follows it. Never describe
a provider, a capability or a sign-in step that the code does not have, and
never present a planned provider as available.

All documentation is in English and written for non-technical users, except
`docs/ADDING_A_PROVIDER.md` and `docs/ARCHITECTURE.md`, which are for
developers. Keep it short: Nuage aims to be a simple application.

## 1. Inventory the providers from the code

Read, do not guess:

- `crates/providers/Cargo.toml`: one feature per provider. A provider in
  `default` is **shipped**; one with a feature outside `default` is
  **experimental**.
- `crates/providers/src/registry.rs`: `with_enabled_providers()` gives the
  order shown to the user. Use the same order everywhere.
- `crates/providers/src/<id>.rs`, for each provider:
  - `id()`, `display_name()`, `rclone_backend()`;
  - `auth()`: `OAuthBrowser { custom_client }` (sign in in the browser;
    `Required` means the user creates their own OAuth client),
    `Credentials { fields }` (a form) or `None`;
  - `static_capabilities()`: quota, public_link, server_side_copy,
    server_side_move, trash, empty_trash, native_docs, hashes;
  - `client_guide()`: the steps and URLs of the guided client setup;
  - `native_docs()`: how documents like Google Docs appear;
  - the module doc comment: scope limits (e.g. "My Drive only").
- `crates/core/src/capabilities.rs` and `crates/core/src/provider.rs`: the
  meaning of each field, and any field added since the last update.
- The `Provider` trait in `crates/providers/src/lib.rs`: its methods, for
  the developer guide.
- `docs/brief.md`, sections "Périmètre par version" and "Jalons": which providers are
  planned, and for which version. Only these may be listed as *planned*.

Write the inventory down (one line per provider) before editing anything.

## 2. Update each document

Check every item; edit only what is out of date.

1. **`docs/PROVIDERS.md`** (create it if missing). For users. Exactly this
   shape:

   ```markdown
   # Providers

   | Provider | Status | Sign-in | Quota | Share link | Trash |
   |---|---|---|---|---|---|
   | Google Drive ("My Drive") | available | browser, with [your own Google client](GOOGLE_OAUTH.md) | yes | yes | yes |
   | Local folder | for tests | none | yes | no | no |
   | OneDrive | planned (v2) | | | | |
   ```

   Then one short section per shipped provider: what it shows (scope
   limits), how special documents appear, anything the user must do once.
   Status is `available` (in `default`), `experimental` (feature outside
   `default`), `for tests` (the `local` provider) or `planned (vX)`
   (brief only). Capability columns come from `static_capabilities()`; say
   in one line under the table that Nuage hides actions a provider does not
   support.
2. **`README.md`**: the single "Providers:" line under "What it does" names
   the available providers, then "… later" for planned ones, and links to
   `docs/PROVIDERS.md`. Do not add a table to the README.
3. **Client guides** (`docs/GOOGLE_OAUTH.md`, and one per provider whose
   `client_guide()` is not empty, named `docs/<PROVIDER>_OAUTH.md`): the
   numbered steps match `client_guide()` one for one, same order, same URLs.
   If they differ, the code wins unless the user says otherwise; then fix
   the code and the doc together (the step titles are shown in the setup
   screen). The comment above `client_guide()` names the doc file.
4. **`docs/SERVER.md`**: the "Connect …" section covers every available
   provider that needs sign-in on a headless server (`--no-browser`,
   `--token-file`, or `--param key=value` for `Credentials` fields).
5. **`docs/CLI.md`**: the "Accounts" examples show `nuage account add <id>`
   with the real options of each available provider.
6. **`docs/TROUBLESHOOTING.md`**: provider-specific problems still match
   the code (e.g. `.link.html` shortcuts if `native_docs()` still uses them).
7. **`docs/ADDING_A_PROVIDER.md`** (create it if missing). For developers:
   the files to touch (module, feature in `Cargo.toml`, line in the
   registry, user docs from this list), every `Provider` method with one
   line each, and the rule that `engine`, `ui`, `tray` and `cli` are not
   modified. Keep it in sync with the trait.
8. **`docs/ARCHITECTURE.md`**, section "Providers": only if the trait or
   the capability model changed.
9. **`data/io.github.felixilgatto.Nuage.metainfo.xml`**: the description
   and the `<keyword>`s name the available providers only.
10. **`CHANGELOG.md`**: under `## [Unreleased]` (create the section if
    needed), one line per user-visible provider change.
11. **Packages**: a new user guide needed on a server (like
    `GOOGLE_OAUTH.md`) is also listed in `scripts/build-native.sh`,
    `build-aux/native/install.sh` and the `assets` of
    `crates/daemon/Cargo.toml`.
12. **Messages that link to a guide** (`grep -rn "docs/" crates`): the
    links point to files that exist.

## 3. Verify

```sh
# Every relative link in the docs points to an existing file.
for f in README.md docs/*.md; do
  grep -o '](\([^)#]*\.md\)' "$f" | sed 's/](//' | while read -r l; do
    [ -e "$(dirname "$f")/$l" ] || echo "$f: broken link $l"
  done
done
# Provider ids in the code vs. the docs.
grep -rhn 'fn id(&self)' -A1 crates/providers/src | grep -o '"[a-z0-9]*"'
cargo test -p nuage-providers
```

Then reread `docs/PROVIDERS.md` against the inventory from step 1, row by
row.

## 4. Report

Tell the user, in their language: which documents changed and why, any
mismatch you fixed in the code (client guide steps), and anything you could
not decide (e.g. a provider in the code but not in the brief). Do not commit
unless asked.
