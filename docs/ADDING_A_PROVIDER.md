# Adding a provider

rclone does the I/O for every backend, so a provider is mostly declarative.
Adding one touches `crates/providers` and the documentation only: **do not
modify `engine`, `ui`, `tray` or `cli`**. If a provider seems to need it,
the abstraction is missing something; extend `nuage-core` (e.g.
`Capabilities`) instead, and discuss it first.

## Files to touch

1. `crates/providers/src/<id>.rs`: a type implementing `Provider` (see
   `gdrive.rs` and `local.rs`), with its unit tests.
2. `crates/providers/Cargo.toml`: a feature named after the id. Add it to
   `default` only when the provider is ready for users: the engine (and so
   `nuaged`) builds the providers of that list.
3. `crates/providers/src/lib.rs`: the `#[cfg(feature = "<id>")] pub mod`
   line.
4. `crates/providers/src/registry.rs`: one line in
   `with_enabled_providers()`, at the place it should appear in the setup.
5. The user documentation: run the `provider-docs` skill
   (`.claude/skills/provider-docs`), which updates `docs/PROVIDERS.md`, the
   README, the sign-in guides and the rest.

## The `Provider` trait

| Method | What it gives |
|---|---|
| `id()` | stable identifier, used in the CLI (`nuage account add <id>`) and the configuration |
| `display_name()` | name shown to the user |
| `icon_name()` | icon name from the icon theme |
| `rclone_backend()` | rclone backend type, e.g. `drive` |
| `auth()` | `OAuthBrowser { custom_client }`, `Credentials { fields }` (a form) or `None` |
| `config_params(input)` | parameters of rclone's `config/create`, built from what the user entered |
| `static_capabilities()` | `Capabilities` known in advance; merged at runtime with `operations/fsinfo` |
| `recommended_vfs_opts()` | VFS options for the mounts of this provider |
| `native_docs()` | how provider-native documents appear, e.g. web shortcuts |
| `client_guide()` *(optional)* | steps and URLs to create the user's own OAuth client; shown in the setup |
| `sub_drives()` *(optional)* | other drives its accounts reach (rclone listing command, selecting parameter), each added as an account of its own |
| `client_id_suffix()` *(optional)* | end of a valid client ID, to check what the user pastes |
| `root(input)` *(optional)* | folder of the remote the account shows (e.g. the S3 bucket); empty for the whole remote |
| `validate(input)` *(optional)* | checks the input before anything is created; the error says what to fix |
| `choice_rank(question, choice)` *(optional)* | order in which to try the choices rclone offers for a question `config_params()` does not answer (e.g. the OneDrive drive); lower first |
| `info()` *(provided)* | the description sent to clients; do not override |

Helpers in `lib.rs` for the usual cases: `oauth_params()` (own client and
token, for an OAuth provider using rclone's client by default), `field()`
(a form field), `credential()` (a form value, trimmed, `None` when blank)
and `check_web_address()` (an `https://` address, for `validate()`).

A `Credentials` provider gets a connection check for free: the engine lists
the account root right after `config/create` and refuses the account with a
message naming the fields the user filled in.

## Rules

- Interfaces hide or refuse actions according to `Capabilities`, never
  according to the provider id. Declare capabilities honestly.
- Error messages from `validate()` are for non-technical users and say what
  to do.
- A provider whose `client_guide()` is not empty gets a guide in `docs/`
  with the same steps, in the same order, with the same URLs.
- Test it end to end with `cargo test` and, if it needs no account, add it
  to the end-to-end tests like `local`.
