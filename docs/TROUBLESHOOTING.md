# Troubleshooting

- **No Nuage icon in the top bar (Ubuntu)**: enable the "Ubuntu
  AppIndicators" extension in the Extensions app. Without it, Nuage keeps
  its window open instead.
- **`nuage` says the daemon is unreachable (exit code 3)**: start it with
  `systemctl --user start nuaged` (or `nuaged &` during development).
- **"mounting is not supported when running from snap"**: the rclone snap
  cannot mount folders. Install rclone from <https://rclone.org/install/>
  and set `NUAGE_RCLONE` to its path.
- **A Google Doc opens in the wrong application** (e.g. Thunderbird): Docs,
  Sheets and Slides are `.link.html` shortcuts, opened by your default
  application for web pages. Make your browser that application, e.g.
  `xdg-mime default firefox.desktop text/html` (or in Settings → Apps →
  Default apps).
- **Google asks you to sign in every week**: your OAuth app is still in
  *Testing*; publish it (step 4 of [GOOGLE_OAUTH.md](GOOGLE_OAUTH.md)), then
  run `nuage account reconnect "Google Drive"`.
- **"… refused the connection" when adding an account** (Nextcloud,
  WebDAV, S3, Proton Drive): one of the values is wrong, and nothing was
  added. Common
  causes:
  - Nextcloud with two-factor authentication accepts only an app password
    (Personal settings → Security → Create new app password).
  - An S3 key limited to one bucket (usual with Cloudflare R2 and Backblaze
    B2) needs that bucket's name in **Bucket**.
  - Amazon S3 outside `us-east-1` may need its **Region**.
- **Changes made on another device show up late**: Nextcloud, WebDAV, S3,
  pCloud and Proton Drive do not announce changes, so Nuage looks for them every minute.
  `nuage refresh` shows them at once.
- **Proton Drive stops working and asks to sign in again**: remove the
  account and add it again; Proton ended the session.
- **The OneDrive "Personal Vault" folder cannot be opened**: Microsoft does
  not let other applications open it. Use the OneDrive website or app for
  the files it holds.
- **The daemon log**: `journalctl --user -u nuaged`, or the daemon's standard
  error during development (`NUAGE_LOG=debug` for more detail).
