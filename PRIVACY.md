# Privacy Policy

_Last updated: 2026-11-05_

Nuage is an open-source desktop and command-line application that mounts your cloud storage (Google Drive,
OneDrive, Nextcloud, an S3 bucket…) as a folder on your own computer. It runs entirely on your device.

## What Nuage accesses

Nuage uses the access you give it to a storage account only to:

- list, download, upload, move, rename and delete the files you work with in the mounted folder or in the app;
- show your storage quota, and create share links when you ask for one.

It accesses files only, never another service of the provider (no mail, calendar, contacts or photos).

- **Google Drive**: Nuage asks Google for access to your Drive files (`https://www.googleapis.com/auth/drive`),
  through an OAuth client you create yourself.
- **OneDrive, Dropbox, pCloud**: you sign in on the provider's page, through the sign-in app of rclone, the
  open-source tool Nuage is built on. For OneDrive, Nuage asks for your files only (`Files.ReadWrite.All` and
  `offline_access`).
- **Nextcloud, WebDAV, S3, Proton Drive**: Nuage uses the address, user name and password or access key you
  enter, to reach the server you chose.

## What Nuage stores, and where

- Your sign-in tokens, passwords, access keys and settings are stored **only on your computer**, in the app's
  configuration directory. When a system keyring is available, the configuration is encrypted with a password
  kept in that keyring; otherwise it is readable by your user only.
- File contents are cached **only on your computer**, within the cache size limit you configure.

## What Nuage does not do

- Nuage has **no server**. Your files, tokens, passwords and account information are never sent to the
  developers or to any third party. Data flows only between your computer and the storage provider or server
  you connected.
- Nuage does not collect analytics, telemetry or crash reports.
- Nuage does not sell, share or transfer your data, and does not use it for advertising or to train AI models.

For Google accounts: Nuage's use and transfer of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Removing access

Remove the account in Nuage (this deletes the local tokens, passwords, cache and settings for that account).
You can also revoke access at any time on the provider's side: Google <https://myaccount.google.com/connections>,
Microsoft <https://account.live.com/consent/Manage>, Dropbox <https://www.dropbox.com/account/connected_apps>,
or by deleting the app password or access key you created for Nuage.

## Contact

Questions about this policy: open an issue at <https://github.com/felixilgatto/nuage/issues>.
