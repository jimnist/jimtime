# SMTP

[ADR-0008](../../adr/0008-invoice-lifecycle.md)

## What

`invoice finalize` and `invoice send` email the PDF through `lettre` (async, rustls with the `ring` provider, so no cmake or OpenSSL at build time).

## Auth

`[email]` in the config holds host, port, `security`, username and addresses.
The password is only `$JIMTIME_SMTP_PASSWORD` (ADR-0003).
With Gmail or Fastmail, use an app password, not the account password.

| Provider | host | port | security |
|---|---|---|---|
| Fastmail | `smtp.fastmail.com` | 465 | `tls` |
| Gmail / Google Workspace | `smtp.gmail.com` | 465 | `tls` |
| Microsoft 365 | `smtp.office365.com` | 587 | `starttls` |
| A relay on this machine (Proton Bridge, postfix, Mailpit) | `localhost` | as configured | `none` |

## Gotchas

- `security = "none"` is refused for any host but localhost, so a password never crosses a network in the clear.
- Everything that can stop a send (config, password, addresses) is checked **before** finalize writes anything, so a typo never burns an invoice number.
- A send that fails after finalizing leaves the invoice issued and unsent; `invoice send <number>` retries.
- Bcc recipients are delivered but never appear in the headers.
- The subject and body are MiniJinja templates with the invoice context; printing an undefined variable is an error, not a blank.
- E2E testing without a real account: run a local SMTP sink (e.g. Mailpit, or `aiosmtpd` with `auth_require_tls=False`) and point `[email]` at `localhost` with `security = "none"`.
