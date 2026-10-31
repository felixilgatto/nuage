# Privacy Policy

_Last updated: 2026-10-31_

Nuage is an open-source desktop and command-line application that mounts your cloud storage (such as Google Drive)
as a folder on your own computer. It runs entirely on your device.

## What Nuage accesses

When you connect a Google account, Nuage asks Google for access to your Google Drive files
(`https://www.googleapis.com/auth/drive`). It uses this access only to:

- list, download, upload, move, rename and delete the files you work with in the mounted folder or in the app;
- show your storage quota.

Nuage does not access any other Google service (no Gmail, Calendar, Contacts or Photos).

## What Nuage stores, and where

- Your OAuth tokens and settings are stored **only on your computer**, in the app's configuration directory.
  When a system keyring is available, the configuration is encrypted with a password kept in that keyring.
- File contents are cached **only on your computer**, within the cache size limit you configure.

## What Nuage does not do

- Nuage has **no server**. Your files, tokens and account information are never sent to the developers
  or to any third party. Data flows only between your computer and Google.
- Nuage does not collect analytics, telemetry or crash reports.
- Nuage does not sell, share or transfer your data, and does not use it for advertising or to train AI models.

Nuage's use and transfer of information received from Google APIs adheres to the
[Google API Services User Data Policy](https://developers.google.com/terms/api-services-user-data-policy),
including the Limited Use requirements.

## Removing access

Remove the account in Nuage (this deletes the local tokens, cache and settings for that account), and revoke
access at any time from your Google Account: <https://myaccount.google.com/connections>.

## Contact

Questions about this policy: open an issue at <https://github.com/felixilgatto/nuage/issues>.
