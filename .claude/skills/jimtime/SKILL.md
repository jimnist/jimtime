---
name: jimtime
description: Log, review, approve, and push billable time with the jimtime CLI. Use when the user wants to track time on the current work, or review/approve/push their hours to Harvest.
disable-model-invocation: true
---

# jimtime

`jimtime` is the CLI that owns time tracking - the store, the repo→Harvest mapping, approval, dedup, and the Harvest push. The CLI is the product; you are the assistant that summarizes the user's work and calls it. Never reimplement its logic.

Run commands from the git repo the work happened in - the mapping is keyed on the repo's toplevel path. Data lives in `$JIMTIME_HOME`; credentials are in the environment.

## Logging time — `jimtime add`

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

## Reviewing — `jimtime review`

```
jimtime review [--today | --week | --last-week | --date YYYY-MM-DD | --from … --to …]
```
Summarize totals, needs-review, unapproved, and eligible-to-push for the user. Default range is today.

## Approving — `jimtime approve` (user gate)

Only when the user explicitly approves. Preview first, then apply:
```
jimtime approve <range> [--client … --project …]     # preview what would be approved
jimtime approve <range> [filters] --yes               # apply; also clears needs-review
```

## Pushing to Harvest — `jimtime harvest` (user gate)

Only on an explicit instruction to push. **Always dry-run first, show it, then push:**
```
jimtime harvest dry-run <range>
jimtime harvest push <range>     # creates REAL billable entries in Harvest; requires a clear go
```
Push is idempotent - already-imported entries are skipped, so re-running is safe.

## Reference

- `jimtime status` / `jimtime map` - the current repo's Harvest mapping
- `jimtime today [--create]` - today's log
- `jimtime harvest projects | clients | tasks --project <id>` - browse Harvest to build the mapping (`$JIMTIME_HOME/config/harvest-projects.json`)

## Safety

- Never approve or push without an explicit user instruction.
- Time is stored exactly; never round it yourself.
- Keep notes concise and invoice-friendly.
