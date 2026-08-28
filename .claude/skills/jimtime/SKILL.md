---
name: jimtime
description: Log, review, approve, and push billable time with the jimtime CLI. Use when the user wants to track time on the current work, or review/approve/push their hours to Harvest.
disable-model-invocation: true
---

# jimtime

`jimtime` is the CLI that owns time tracking - the store, the repo→Harvest mapping, approval, dedup, and the Harvest push. The CLI is the product; you are the assistant that summarizes the user's work and calls it. Never reimplement its logic.

Run commands from the git repo the work happened in - the mapping is keyed on the repo's toplevel path. Data lives in `$JIMTIME_HOME`; credentials are in the environment. Billing days are anchored to `$JIMTIME_TZ` (default `America/Los_Angeles`), not the machine clock, so never compute dates yourself - let the CLI resolve "today".

## Logging time - `jimtime add`

When the user asks to log time for work in this session:

1. Summarize what was done into a concise, invoice-friendly note - what shipped or changed, not a play-by-play.
2. Estimate hours **conservatively**. Do not invent precise times. If unsure, round down and add `--needs-review`.
3. Run from the working repo:
   ```
   jimtime add --hours <H> --notes "<note>" [--needs-review] [--task <alias>] [--billable no]
   ```
   Use `--from HH:MM --to HH:MM` instead of `--hours` only if the user gives real clock times.
4. Show the user exactly what you logged.

Rules:
- Conservative estimates; never pad.
- `--needs-review` whenever the estimate is uncertain.
- One entry per distinct chunk of work; use `--task <alias>` for a non-default task.
- Never `approve`. Never `push` unless the user explicitly tells you to.

## Reviewing - `jimtime review`

```
jimtime review [<range>] [--pending]
```
Lists each entry with its ID and status (`●` unapproved, `○` approved, `[needs review]`, `[imported]`) plus totals. `--pending` shows only unapproved entries - use it to show the user what's outstanding before approving. Range flags: `--today | --week | --last-week | --date YYYY-MM-DD | --from … --to …` (default today). If an entry is wrong, the user can edit the day's JSON in `$JIMTIME_HOME/entries/…` directly.

## Approving - `jimtime approve` (user gate)

Only when the user explicitly approves. Approval is per entry; `approve` sweeps every unapproved entry in scope:
```
jimtime approve <range> [--client … --project …]        # approves all except needs-review
jimtime approve <range> --except <id> [--except <id>]   # …but hold these
jimtime approve <range> --include-needs-review          # also approve flagged ones
jimtime approve <range> --only <id> [--only <id>]       # approve just these (bypasses the hold)
```
`needs-review` entries are held by default (it prints which). Approving clears the flag. Run `jimtime review --pending <range>` first and show the user.

To take an approval back, `jimtime unapprove` mirrors the same flags (`--except`, `--only`). It refuses entries already pushed to Harvest.

## Pushing to Harvest - `jimtime harvest` (user gate)

Only on an explicit instruction to push. **Always dry-run first, show it, then push:**
```
jimtime harvest dry-run <range>
jimtime harvest push <range>     # creates REAL billable entries in Harvest; requires a clear go
```
Re-running skips entries that already saved a Harvest id, so it won't double-push those. If a push *errors* (e.g. a timeout), the entry may still have been created in Harvest - tell the user to check there before re-running.

## Reference

- `jimtime status` / `jimtime map` - the current repo's Harvest mapping
- `jimtime today [--create]` - today's log
- `jimtime report <range> [--billable-only]` - markdown export to paste/share
- `jimtime harvest projects | clients | tasks --project <id>` - browse Harvest to build the mapping (`$JIMTIME_HOME/config/harvest-projects.json`)
- `jimtime harvest uninvoiced [--from … --to …] [--with-expenses]` - read-only: what each client owes for tracked-but-not-invoiced time, largest first (defaults to the last two months)

## Safety

- Never approve or push without an explicit user instruction.
- Time is stored exactly; never round it yourself.
- Keep notes concise and invoice-friendly.
