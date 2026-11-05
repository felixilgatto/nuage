# Create your own Google OAuth client

Google Drive needs an OAuth client of your own: rclone's shared Google
client is being retired, and a client shared by all Nuage users would need
a costly Google audit. Creating yours takes about five minutes and is free. You do it
once; your other Google accounts reuse it.

The desktop app walks you through these same steps.

1. **Create a project**: open <https://console.cloud.google.com/projectcreate>,
   name it (e.g. "Nuage"), click **Create**, and make sure it is selected at
   the top of the page.
2. **Enable the Drive API**: open
   <https://console.cloud.google.com/apis/library/drive.googleapis.com> and
   click **Enable**.
3. **Describe the app**: open <https://console.cloud.google.com/auth/branding>,
   click **Get started**, give an app name (e.g. "Nuage for me") and your
   e-mail address, choose **External** as audience, and finish.
4. **Publish it**: open <https://console.cloud.google.com/auth/audience> and
   click **Publish app**, then confirm. This matters: while the app is in
   *Testing*, Google makes you sign in again every 7 days. Google does not
   review a personal app; it only warns, when you sign in, that the app is
   not verified (click **Advanced**, then **Go to …**: it is your own app).
5. **Create the client**: open <https://console.cloud.google.com/auth/clients>,
   click **Create client**, choose the type **Desktop app**, and create it.
   Copy the **Client ID** and the **Client secret**.

Nuage asks Google for access to your Drive files only (`drive` scope), no
other Google service.

## Connect Google Drive from the command line

On a computer with a web browser:

```sh
nuage account add gdrive --client-id <client ID> --client-secret <client secret>
```

Your browser opens; sign in and allow access. The account is mounted at
`~/Cloud/Google Drive` (choose another folder with `--mount-point`). Google
Docs, Sheets and Slides appear as `.link.html` shortcuts that open in the
browser.

On a server without a browser, see [SERVER.md](SERVER.md#connect-google-drive).

If Google later asks you to sign in again (for example after you changed
your password), run `nuage account reconnect "Google Drive"`.

## Where your access is stored

The sign-in tokens are in rclone's configuration. On a desktop, it is
encrypted with a password kept in your keyring. On a server without a
keyring, it is not encrypted but readable by your user only; `nuage status`
says so.
