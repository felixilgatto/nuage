# Providers

| Provider | Status | Sign-in | Quota | Share link | Trash |
|---|---|---|---|---|---|
| Google Drive ("My Drive", shared drives) | available | browser, with [your own Google client](GOOGLE_OAUTH.md) | yes | yes | yes |
| OneDrive (personal) | available | browser | yes | yes | yes |
| Dropbox | available | browser | yes | yes | yes |
| pCloud | available | browser | yes | yes | yes |
| Proton Drive | experimental | Proton address, password and two-factor code | yes | no | yes |
| Nextcloud | available | server address, user name and app password | yes | no | yes |
| WebDAV | available | WebDAV address, user name and password | no | no | no |
| S3 bucket storage (Amazon S3 and compatible) | available | access key | no | no | no |
| Local folder | for tests | none | yes | no | no |

Nuage hides the actions a provider does not support. For the providers you
sign in to with a form, Nuage tries the connection before adding the
account, and says what to check if it fails.

## Google Drive

- Shows "My Drive". Each shared drive can be added as an account of its
  own, with its own folder, without signing in again: in the app, "+" →
  Google Drive → the shared drive; on the command line,
  `nuage account drives "Google Drive"` then
  `nuage account add gdrive --from "Google Drive" --drive "<name>"`.
  "Shared with me" comes later.
- Google Docs, Sheets and Slides appear as `.link.html` shortcuts that open
  in your web browser.
- Done once: create your own Google client, see
  [GOOGLE_OAUTH.md](GOOGLE_OAUTH.md). The desktop app guides you through
  it; your other Google accounts reuse it.

## OneDrive

- Shows the personal OneDrive of the Microsoft account you sign in with.
  SharePoint and OneDrive for work or school come later.
- Office documents are regular files: they open in your usual applications.
- Nothing to set up: sign in in your browser.
- The **Personal Vault** ("Coffre-fort" in French) shows as a folder that
  cannot be opened: Microsoft only opens it in its own apps.

## Dropbox

- Shows the whole Dropbox of the account you sign in with.
- Nothing to set up: sign in in your browser.
- Deleted files can be restored from dropbox.com.

## pCloud

- Shows the whole pCloud of the account you sign in with, in the US or the
  EU region (found by the sign-in).
- Nothing to set up: sign in in your browser.

## Proton Drive

Experimental: not in the regular builds yet (rclone's Proton Drive support
is in beta). Build with the `protondrive` feature to try it.

- Sign in with your Proton address and password, and the six-digit code of
  your authenticator app if two-factor authentication is on.
- Nuage keeps the session, not the code. If Proton ends the session, remove
  the account and add it again.
- Accounts with a separate mailbox password are not supported yet.

## Nextcloud

- Shows your Nextcloud files. Large files are uploaded in pieces.
- Enter your server's address (e.g. `https://cloud.example.com`) or the
  WebDAV address shown in Nextcloud, under Files → Settings. Use the second
  one if you sign in with your e-mail address.
- Use an app password, required with two-factor authentication: in
  Nextcloud, Personal settings → Security → Create new app password.
- Changes made elsewhere appear within a minute.

## WebDAV

- Any WebDAV server: ownCloud, a NAS, Fastmail… For Nextcloud, choose
  Nextcloud instead.
- Enter the full WebDAV address given by the server, with its `https://`.
- Changes made elsewhere appear within a minute.

## S3 bucket storage

- Amazon S3 and S3-compatible storage: Cloudflare R2, Backblaze B2, Wasabi,
  MinIO, Scaleway…
- Name a bucket to show only that bucket (or `bucket/folder`); this works
  with a key limited to that bucket. Leave it empty to see every bucket as
  a folder: the key must then be allowed to list buckets.
- For Amazon S3, leave the endpoint empty. Otherwise enter the endpoint your
  service gives (e.g. `https://<account>.r2.cloudflarestorage.com`) and, if
  it asks for one, the region.
- Changes made elsewhere appear within a minute.

## Local folder

A folder of this computer, shown in another folder. It is there to test
Nuage without a cloud account.
