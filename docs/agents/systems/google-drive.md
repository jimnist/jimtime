# Google Drive (optional)

[ADR-0010](../../adr/0010-invoice-pdfs-to-cloud-folders.md)

## What

After an invoice is finalized and sent, its PDF is uploaded into `[cloud.google_drive] folder` (a `/`-separated path under My Drive, created if missing) as `Invoice <number>.pdf`.
Re-uploading replaces the file's content rather than adding a second copy.

## One-time setup

1. https://console.cloud.google.com → create (or pick) a project.
2. **APIs & Services → Library**: enable the **Google Drive API**.
3. **OAuth consent screen**: *External*, fill in the app name and your email, add the scope `.../auth/drive.file`, and add yourself as a test user.
   Then **publish** the app ("In production").
   In *Testing*, Google expires refresh tokens after 7 days, so the login would silently stop working a week later.
   `drive.file` is a non-sensitive scope, so a personal app can be published without Google's verification; the consent screen just shows an "unverified app" notice.
4. **Credentials → Create credentials → OAuth client ID → Desktop app**.
   Put the client id in the config and the client secret in the environment:

```toml
[cloud.google_drive]
client_id = "1234-abc.apps.googleusercontent.com"
folder = "Business/Invoices"
```

```sh
export JIMTIME_GDRIVE_CLIENT_SECRET="GOCSPX-..."
```

Then `jimtime cloud login google-drive`: it opens Google, you approve, and the browser lands back on a local page that says jimtime is connected.

## Auth

PKCE with a loopback redirect to `http://127.0.0.1:<random port>`, which Google allows for desktop clients, plus a `state` check.
`access_type=offline&prompt=consent` makes Google return a refresh token, stored in the OS keychain (service `jimtime`, account `google-drive`).

## Gotchas

- `drive.file` means jimtime can see only the folders and files it created itself. A folder of the same name made by hand in the Drive UI is invisible to it, so jimtime makes its own.
- Google calls a desktop client's secret non-confidential, but it is still kept out of files per ADR-0003.
- Folder and file names in the `q` search are escaped (`'` and `\`).
