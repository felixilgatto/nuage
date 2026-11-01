# Providers

| Provider | Status | Sign-in | Quota | Share link | Trash |
|---|---|---|---|---|---|
| Google Drive ("My Drive") | available | browser, with [your own Google client](GOOGLE_OAUTH.md) | yes | yes | yes |
| Local folder | for tests | none | yes | no | no |
| OneDrive and others | planned (v2) | | | | |

Nuage hides the actions a provider does not support.

## Google Drive

- Shows "My Drive" only. Shared drives and "Shared with me" come later.
- Google Docs, Sheets and Slides appear as `.link.html` shortcuts that open
  in your web browser.
- Done once: create your own Google client, see
  [GOOGLE_OAUTH.md](GOOGLE_OAUTH.md). The desktop app guides you through
  it; your other Google accounts reuse it.

## Local folder

A folder of this computer, shown in another folder. It is there to test
Nuage without a cloud account.
