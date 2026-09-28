# Dropbox (optional)

[ADR-0010](../../adr/0010-invoice-pdfs-to-cloud-folders.md)

## What

After an invoice is finalized and sent, its PDF is uploaded to `[cloud.dropbox] folder` as `Invoice <number>.pdf`, replacing a file of the same name (`/2/files/upload`, `mode: overwrite`).

## One-time setup

1. https://www.dropbox.com/developers/apps → **Create app** → *Scoped access*.
2. Access type: **App folder** is the tidy choice (the app sees only `Apps/<app name>/`, and `folder = "/Invoices"` is relative to it); *Full Dropbox* works too, with `folder` an absolute path.
3. **Permissions** tab: tick `files.content.write`, then **Submit**.
4. Copy the **App key** into the config. No app secret is needed: the login uses PKCE.

```toml
[cloud.dropbox]
app_key = "abc123..."
folder = "/Invoices"
```

Then `jimtime cloud login dropbox`: it opens Dropbox, you approve, and paste the code it shows.

## Auth

PKCE "no redirect" flow (`token_access_type=offline`), which returns a long-lived refresh token.
It is stored in the OS keychain (service `jimtime`, account `dropbox`); every upload trades it for a short-lived access token.
`jimtime cloud logout dropbox` removes it.

## Gotchas

- Dropbox needs redirect URIs registered exactly, so a random loopback port cannot be used; that is why this flow pastes a code.
- The `Dropbox-API-Arg` header must be ASCII; non-ASCII in the path is sent as `\uXXXX` escapes.
- Changing the app's permissions invalidates existing tokens; log in again.
