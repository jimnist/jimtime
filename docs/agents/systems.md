# Systems

External systems this app depends on.
Each has a file under `systems/`: what it is used for, how it is authenticated, and the gotchas.

## Git (the data repo)

`$JIMTIME_HOME` as its own private repo, pulled, committed and pushed around every write, with a custom merge driver for day files.
Always on when the data home is its own repo; nothing to sign up for.

Read more: `systems/git-data-repo.md`

## SMTP

Emailing finalized invoices. Any provider; the password is `$JIMTIME_SMTP_PASSWORD`.

Read more: `systems/smtp.md`

## Chrome (headless)

Printing invoice HTML to PDF. A runtime dependency, found automatically.

Read more: `systems/chrome.md`

## Harvest (optional, off by default)

Pushing approved time entries; reading clients/projects/tasks and the uninvoiced report.
Env credentials `HARVEST_ACCESS_TOKEN`, `HARVEST_ACCOUNT_ID`.

Read more: `systems/harvest.md`

## Dropbox and Google Drive (optional)

A copy of each finalized invoice PDF in a folder. OAuth with PKCE; refresh tokens in the OS keychain.

Read more: `systems/dropbox.md`, `systems/google-drive.md`
