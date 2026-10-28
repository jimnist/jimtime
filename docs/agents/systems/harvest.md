# Harvest (optional)

[ADR-0005](../../adr/0005-approve-and-push-stay-separate.md), [ADR-0006](../../adr/0006-harvest-is-optional.md)

## What

Harvest v2 API: create and delete time entries (`harvest push` / `unpush`, `approve --push`), list clients, projects and task assignments, and read `/v2/reports/uninvoiced`.
Configured in `config/harvest.toml` (ADR-0006). Off unless `enabled = true` there; while off, every `harvest` subcommand fails before any credential is read.
Separately, `numbering = true` reads `/v2/invoices` (numbers only) so jimtime's invoice numbers continue Harvest's; it works with pushing off.
`invoice import-harvest` (ADR-0011) reads `/v2/users/me`, `/v2/company`, `/v2/invoices`, `/v2/time_entries?user_id=`, `/v2/clients/{id}`, and each invoice's PDF from `https://{company.full_domain}/client/invoices/{client_key}.pdf` (the client-facing link; no auth, and the API has no PDF endpoint).
A time entry's `invoice: {id, number}` is what links time to the invoice that billed it.

## Auth

`HARVEST_ACCESS_TOKEN` and `HARVEST_ACCOUNT_ID` from the environment (ADR-0003); optional `HARVEST_USER_AGENT`.
Tokens: https://id.getharvest.com/developers

## Gotchas

- Push resolves project and task ids from the section first, then from harvest.toml, so ids added there later apply to time logged before.
  `harvest dry-run` flags sections that would fail for a missing id.
- The hours-based create only works for accounts that track by duration; push checks `/v2/company` first.
- The uninvoiced report rejects ranges wider than 365 days; wider ranges are split into disjoint windows.
- A 404 on delete is success (already gone); an invoiced or locked entry cannot be deleted and stays linked.
- Harvest invoicing and jimtime invoicing are independent. `invoice draft`/`finalize` warn when the time being invoiced was also pushed to Harvest.
