# jimtime - build spec

Personal CLI for tracking billable time per git repo, reviewing/approving it, invoicing approved billable time as PDFs from HTML templates, and optionally pushing it to Harvest.
The design was sharpened in grilling sessions (interviewing the spec against the Harvest API docs, then again for invoicing and sync); see `CONTEXT.md` for the glossary and `docs/adr/` for the load-bearing decisions.

## Model

- **Source of truth:** one JSON file per day, `$JIMTIME_HOME/entries/YYYY/MM/YYYY-MM-DD.json`. [ADR-0001, ADR-0002]
  Invoices add a record per invoice, `invoices/YYYY/<number>.json`, beside its PDF. [ADR-0008]
- **Rendering** is on demand and ephemeral: `review`/`today` print to the terminal; Claude Code reads the JSON. No persisted markdown. Invoice PDFs are the one rendered artifact kept, because they are what was sent.
- **Billing timezone:** `$JIMTIME_TZ` (an IANA name) if set, else `America/Los_Angeles`. Billing days are anchored to it regardless of the machine's clock; an unknown or empty value is a hard error, never a silent fallback.
- **Data home:** `$JIMTIME_HOME` if set, else XDG data dir (`~/.local/share/jimtime`). The code carries no personal paths.
- **Data repo:** when the data home is its own git repo, every write pulls first and commits and pushes after; day files merge through a semantic merge driver. [ADR-0009]
- **Secrets** are env-only (`JIMTIME_SMTP_PASSWORD`, `HARVEST_*`, `JIMTIME_GDRIVE_CLIENT_SECRET`), except cloud refresh tokens, which live in the OS keychain. [ADR-0003, ADR-0010]

### Day JSON shape

```json
{
  "date": "2026-07-28",
  "sections": [
    {
      "repo_path": "/Users/jimnist/code/client/acme",
      "client": "acme", "client_name": "Acme",
      "project": "billing-portal", "project_name": "Billing Portal",
      "task": "development", "task_name": "Development",
      "harvest_client_id": 123, "harvest_project_id": 234, "harvest_task_id": 345,
      "entries": [
        {
          "id": "2026-07-28-acme-billing-portal-development-001",
          "hours": 1.25, "billable": true, "approved": true, "needs_review": false,
          "notes": "Implemented webhook retry handling",
          "harvest_time_entry_id": 987,
          "invoice": "2026-004"
        }
      ]
    }
  ]
}
```

A **Section** groups entries by `(repo_path, client, project, task)` keys and is a storage/display grouping only. **Approval is per entry** [ADR-0004]. The `harvest_*` fields appear only when Harvest is used; `invoice` only once the entry is billed. Entry IDs are `YYYY-MM-DD-<client>-<project>-<task>-###` (slugs of the names), the suffix incrementing within a Section.
Legacy files are migrated on load: section-level `approved` is pushed onto the entries, the old `client_id`/`project_id`/`task_id` names are read as the `harvest_*` ids, and missing keys are slugified from the names. `config migrate` rewrites them all once.

## Config

`$JIMTIME_HOME/config/jimtime.toml` [ADR-0006]: `[business]`, `[invoice]`, `[email]`, `[git]`, `[cloud.*]`, `[clients.<key>]` with nested `[clients.<key>.projects.<key>]` (rate, task_rates, default_task, billable), `[tasks.<key>]`, and `[[repos]]` mapping a repo's canonical `git rev-parse --show-toplevel` to a client and project.
`$JIMTIME_HOME/config/harvest.toml` [ADR-0006], only when Harvest is used: `enabled` (pushing, off by default), `numbering`, and `id`s under `[clients.<key>]`, `[clients.<key>.projects.<key>]` and `[tasks.<key>]` (with `name`). Tasks are the union of both files.
Validated at load with `deny_unknown_fields` and cross-reference checks. One repo → one client/project; multiple sections in a day arise only from task overrides.

## Rules

- **Billing:** store exact hours; no rounding on push. Invoice lines are `round2(hours × rate)`, and the total is the sum of the rounded lines. [ADR-0007]
- **Review/approve** operate over a date range *or* a single day (`--today`/`--week`/`--last-week`/`--month`/`--last-month`/`--date`/`--from`+`--to`).
- **Approval** is per entry [ADR-0004]. `approve` sweeps every unapproved entry in scope except `needs-review` ones (held) and `--except <id>`; `--include-needs-review` sweeps those too; `--only <id>` acts on exactly the named ids and bypasses the hold. `unapprove` mirrors it and refuses entries that are pushed to Harvest or on an invoice.
- **Invoicing** [ADR-0008]: `Entry::is_invoiceable` = `approved && billable && invoice.is_none()`. `invoice draft` renders and fingerprints and saves nothing; `invoice finalize` requires interactive approval or `--confirm <fingerprint>`, requires a successful pull, then numbers, locks the entries, writes record + PDF, commits and pushes, emails, and uploads. Send/upload failures leave the invoice issued and name the retry. `invoice void` unlocks the entries and keeps the number used. `invoice paid` records payment; `invoice list` shows paid/open/overdue and the outstanding total. [ADR-0011]
- **Harvest history** [ADR-0011]: `invoice import-harvest` (read-only to Harvest, re-runnable) turns each issued Harvest invoice into a `source: harvest` record with its PDF, keeping Harvest's line items and amount as what was billed; locks local entries Harvest billed; and backfills Harvest-only entries.
- **Harvest** [ADR-0005, ADR-0006]: off by default. Push is explicit only; pushes `approved && billable && !imported` entries (`Entry::is_pushable`); saves each id back immediately; fails loud, no silent partial success. Ids come from the section, else from config.
- **Uninvoiced (Harvest):** `harvest uninvoiced` reads `/v2/reports/uninvoiced` and rolls its per-project rows up by client, per currency; wider-than-365-day ranges are split into disjoint windows.
- **needs_review** is a structured flag on the entry (the View shows it); notes are not polluted with markers.

## Install

`cargo install --path .` puts a `jimtime` binary on `$PATH`. Release binaries are built for macOS only (both arches); every other platform builds from source, which needs no system libraries (rustls with `ring` for both HTTPS and SMTP). Rendering PDFs needs an installed Chromium-family browser at runtime. It runs in the caller's cwd, so `git rev-parse --show-toplevel` resolves the real working repo - no wrapper/alias hack.

## Skill

`/jimtime` - user-invoked only (`disable-model-invocation: true`), symlinked into `~/.claude/skills/`. Drives add/review/approve/invoice/push with human gates on approve, finalize and push. Source lives in this repo under `.claude/skills/jimtime/`.

## Build phases (all complete)

1. **Local logging:** `status`, `map`, `add`, `today`.
2. **Review/approval:** `review` (+`--pending`), `approve` (+`--except`/`--include-needs-review`), `unapprove`.
3. **Harvest:** `harvest dry-run`, `harvest push` (+ `projects`/`clients`/`tasks` lookups, `uninvoiced` balances).
4. **Polish:** `report` markdown export, test suite, task aliases, date shortcuts, the `/jimtime` skill, install docs.
5. **Independence and invoicing:** TOML config with local client/project/task keys and Harvest made optional (`config init|migrate|check`); invoices (`invoice draft|finalize|send|void|paid|list|open|import-harvest`) from HTML templates via headless Chrome, emailed over SMTP; the auto-synced data repo with a semantic day-file merge driver (`data init|status|sync`); invoice PDF uploads to Dropbox and Google Drive (`cloud login|logout|status|upload`).
