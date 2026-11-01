# Command line

Everything the desktop app does can be done with `nuage`, except the
graphical file browser. Run `nuage --help` for the full list.

Every command accepts `--json`. Exit codes:

| Code | Meaning |
|---|---|
| 0 | success |
| 1 | error |
| 2 | incorrect usage |
| 3 | daemon unreachable |
| 4 | authentication required |

Commands: `status [--watch]`, `account add | list | rename | reconnect |
remove`, `mount`, `unmount`, `pause`, `resume`, `refresh`, the file commands
below, `exclude add | remove | list`, `config get | set` and
`daemon status | stop`. Pins (v1.1) answer "not implemented yet" (exit
code 1).

## Accounts

```sh
nuage account add local --param root=/tmp/files      # a local folder (tests)
nuage account add gdrive --client-id <id> --client-secret <secret>
nuage account list
nuage account rename "Google Drive" "Work"
nuage account remove "Work"
```

Google Drive needs your own OAuth client: see
[GOOGLE_OAUTH.md](GOOGLE_OAUTH.md).

## Files

Remote paths are written `<account>:<path>` (`"Google Drive:"` is the
root). They work even when the account is not mounted.

```sh
nuage ls "Google Drive:Documents"
nuage mkdir "Google Drive:Documents/2026"
nuage mv "Google Drive:Documents/draft.odt" "Google Drive:Documents/2026"   # into a folder
nuage mv "Google Drive:Documents/a.odt" "Google Drive:Documents/b.odt"      # rename
nuage cp "Google Drive:Photos" "Other Account:Backup"                      # across accounts
nuage upload report.pdf photos/ "Google Drive:Documents"
nuage download "Google Drive:Documents/2026" ~/Downloads
nuage link "Google Drive:Documents/report.pdf"                             # share link
nuage rm "Google Drive:Documents/old.odt"
nuage rm --recursive "Google Drive:Documents/old"
```

`rm` sends items to the account's trash when it has one; otherwise the
deletion cannot be undone and needs `--yes`. Existing files are never
overwritten by `mv` and `cp`. In `ls`, Google Docs show as
`[web document]`, excluded items as `[not synced]`, files still uploading
as `[uploading]`.

## Exclusions

Items you exclude are neither downloaded nor uploaded, and disappear from
the folder; they stay intact in the cloud.

```sh
nuage exclude add "Google Drive" Archives                 # a folder of the account
nuage exclude add "Google Drive" --glob 'node_modules/'   # a pattern, anywhere
nuage exclude list
nuage exclude remove "Google Drive" Archives
```

Temporary and lock files of office suites are excluded by default (`~$*`,
`.~lock.*#`, `*.tmp`, `.goutputstream-*`); turn that off with
`nuage config set "<account>:default-exclusions" off`. A file you create in
the folder whose name is excluded stays on this computer only, at its place
in the folder. Changing an exclusion briefly remounts the account; if
uploads are pending, add `--yes`.

## Settings

`nuage config get`, `nuage config set <key> <value>`:

| Key | Value |
|---|---|
| `cache-limit` | cache limit of new accounts, e.g. `20G` |
| `<account>:cache-limit` | cache limit of one account |
| `cache-max-age` | how long unused files stay in the cache, e.g. `30d` |
| `<account>:default-exclusions` | `on` or `off`: exclude temporary and lock files |
| `quit-with-pending` | what `nuage daemon stop` does while uploads are pending: `finish-in-background` (default), `ask` or `quit-now` |

Cache settings apply the next time the account is mounted.
