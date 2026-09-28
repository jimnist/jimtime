# Finalized invoice PDFs can be uploaded to Google Drive and/or Dropbox

Git holds and syncs all data (ADR-0009), but a git repo is not a place an accountant, or the user on a phone, browses for PDFs.
So cloud storage has one narrow job: a copy of each finalized invoice PDF in a normal folder.

## Decision

- Both providers are optional and off unless their config section exists: `[cloud.dropbox]` (`app_key`, `folder`) and `[cloud.google_drive]` (`client_id`, `folder`).
- jimtime talks to each provider's HTTP API directly, with no desktop client and no rclone.
- **Auth is a one-time OAuth login** (`jimtime cloud login dropbox|google-drive`) using PKCE.
  Dropbox uses its no-redirect flow (paste the code), because it requires exact redirect URIs and a random loopback port cannot be registered.
  Google uses the loopback redirect that desktop apps are allowed.
  Google Drive access is scoped to `drive.file`: jimtime can see only the folders and files it created.
- **Refresh tokens are stored in the OS keychain** (service `jimtime`), never on disk.
  This amends ADR-0003: the reason for env-only secrets was that an on-disk secret file is a commit-and-sync leak hazard.
  The keychain is not on disk in the data home, so it keeps that property, and a long-lived OAuth token is not something to paste into a shell profile.
  Google's desktop `client_secret` (which Google itself says is not confidential for installed apps) still comes from `$JIMTIME_GDRIVE_CLIENT_SECRET`, following ADR-0003.
- Uploading happens after an invoice is finalized and sent; an existing file of the same name is replaced.
  A failed upload never fails the invoice. It is reported, recorded on the invoice record, and retried with `jimtime cloud upload <number>`.

## Consequences

- The user registers their own Dropbox app and Google OAuth client once; the setup steps are in `docs/agents/systems/`.
- Nothing about the data model depends on the cloud copies; they can be deleted and re-uploaded.
