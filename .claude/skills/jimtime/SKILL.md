---
name: jimtime
description: Log, review, approve, and invoice billable time with the jimtime CLI (and optionally push it to Harvest). Use when the user wants to track time on the current work, review or approve their hours, or draft and send an invoice.
disable-model-invocation: true
---

# jimtime

`jimtime` is the CLI that owns time tracking - the store, the config, approval, invoicing, dedup, the auto-synced data repo, and the optional Harvest push. The CLI is the product; you are the assistant that summarizes the user's work and calls it. Never reimplement its logic, and never edit the data repo with git yourself - jimtime commits and pushes every change.

Run commands from the git repo the work happened in - the mapping is keyed on the repo's toplevel path. Data lives in `$JIMTIME_HOME`; secrets are in the environment. Billing days are anchored to `$JIMTIME_TZ` (default `America/Los_Angeles`), not the machine clock, so never compute dates yourself - let the CLI resolve "today" and the range flags.

## Logging time - `jimtime add`

When the user asks to log time for work in this session:

1. Summarize what was done into a concise, invoice-friendly note - what shipped or changed, not a play-by-play. It appears on the client's invoice.
2. Estimate hours **conservatively**. Do not invent precise times. If unsure, round down and add `--needs-review`.
3. Run from the working repo:
   ```
   jimtime add --hours <H> --notes "<note>" [--needs-review] [--task <key>] [--billable no]
   ```
   Use `--from HH:MM --to HH:MM` instead of `--hours` only if the user gives real clock times. Task keys are in `[tasks]` of the config (`jimtime config check` lists them).
4. Show the user exactly what you logged.

Rules:
- Conservative estimates; never pad.
- `--needs-review` whenever the estimate is uncertain.
- One entry per distinct chunk of work; `--task <key>` for a non-default task.
- Never `approve`, `invoice finalize`, or `harvest push` unless the user explicitly tells you to.

## Reviewing - `jimtime review`

```
jimtime review [<range>] [--pending]
```
Lists each entry with its ID and status (`●` unapproved, `○` approved, `[needs review]`, `[invoice N]`, `[imported]`) plus totals and how many are ready to invoice. `--pending` shows only unapproved entries - use it to show the user what's outstanding before approving. Range flags: `--today | --week | --last-week | --month | --last-month | --date YYYY-MM-DD | --from … --to …` (default today). If an entry is wrong, the user can edit the day's JSON in `$JIMTIME_HOME/entries/…`; `jimtime data sync` then validates and commits the edit.

## Approving - `jimtime approve` (user gate)

Only when the user explicitly approves. Approval is per entry; `approve` sweeps every unapproved entry in scope:
```
jimtime approve <range> [--client … --project …]        # approves all except needs-review
jimtime approve <range> --except <id> [--except <id>]   # …but hold these
jimtime approve <range> --include-needs-review          # also approve flagged ones
jimtime approve <range> --only <id> [--only <id>]       # approve just these (bypasses the hold)
```
`needs-review` entries are held by default (it prints which). Approving clears the flag. Run `jimtime review --pending <range>` first and show the user.

To take an approval back, `jimtime unapprove` mirrors the same flags. It refuses entries already on an invoice or pushed to Harvest.

## Invoicing - `jimtime invoice` (user gate)

Only on an explicit instruction to invoice. **Always draft first, show the user, then finalize with the draft's fingerprint:**
```
jimtime invoice draft --client <key> <range>
jimtime invoice finalize --client <key> <same range> --confirm <fingerprint>   # REAL: numbers, locks, emails the client
```
1. Run the draft. It opens the PDF on the user's screen and prints the total, recipients, a fingerprint, and notes about what was left out (unapproved time, or time also pushed to Harvest - relay those warnings).
2. Tell the user the total and recipients and ask them to check the PDF. Wait for a clear go.
3. Only then run the finalize line the draft printed, with its `--confirm <fingerprint>`. If it says the selection changed, draft again and re-confirm with the user - never work around it.

`finalize` emails the PDF to the client; `--no-send` finalizes without emailing. If the user wants someone extra copied, pass `--cc <address>` to both the draft and the finalize (the draft prints the finalize line with it). If the email or cloud upload fails, the invoice is still issued - tell the user and offer the retry it names (`jimtime invoice send <number>` / `jimtime cloud upload <number>`). `jimtime invoice list` shows invoices with paid/open/overdue status and the outstanding total; `jimtime invoice paid <number> [--date YYYY-MM-DD]` records a payment when the user says one came in; `jimtime invoice void <number>` voids one (its entries become invoiceable again; only on explicit instruction). `jimtime invoice import-harvest` (read-only toward Harvest) brings Harvest's invoices, PDFs and billed time into the store - always run `--dry-run` first and show the user.

## Pushing to Harvest - `jimtime harvest` (optional, user gate)

Only if Harvest is enabled in the config and the user explicitly says to push. **Always dry-run first, show it, then push:**
```
jimtime harvest dry-run <range>
jimtime harvest push <range>     # creates REAL entries in Harvest; requires a clear go
```
Re-running skips entries that already saved a Harvest id. If a push *errors* (e.g. a timeout), the entry may still have been created in Harvest - tell the user to check there before re-running.

## Reference

- `jimtime status` / `jimtime map` - the current repo's mapping, rate, and data-repo sync state
- `jimtime today [--create]` - today's log
- `jimtime report <range> [--billable-only]` - markdown export to paste/share
- `jimtime config check` - validate the config and list clients, projects, tasks
- `jimtime data status` / `jimtime data sync` - data repo state; commit hand edits and sync
- `jimtime cloud status` - which invoice upload targets are ready
- `jimtime harvest uninvoiced` - read-only, Harvest only: what each client owes there

## Safety

- Never approve, finalize, send, void, or push without an explicit user instruction.
- Never finalize without the fingerprint from a draft the user has seen.
- Time is stored exactly; never round it yourself.
- Keep notes concise and invoice-friendly - the client reads them.
