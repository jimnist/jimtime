# jimtime - build spec

Personal CLI for tracking billable time per git repo, reviewing/approving it, and pushing approved billable entries to Harvest.
The design was sharpened in a grilling session (interviewing the spec against the Harvest API docs); see `CONTEXT.md` for the glossary and `docs/adr/` for the load-bearing decisions.

## Model

- **Source of truth:** one JSON file per day, `$JIMTIME_HOME/entries/YYYY/MM/YYYY-MM-DD.json`. The only persisted artifact. [ADR-0001, ADR-0002]
- **Rendering** is on demand and ephemeral: `review`/`today` print to the terminal; Claude Code reads the JSON. No persisted markdown.
- **Data home:** `$JIMTIME_HOME` if set, else XDG data dir (`~/.local/share/jimtime`). Jim sets `JIMTIME_HOME=~/code/jimnist/bigbrain/time_tracking`. The code carries no personal paths.
- **Committed to git** (in bigbrain): the entries JSON and `config/harvest-projects.json`. Harvest tokens are env-only, never on disk.

### Day JSON shape

```json
{
  "date": "2026-07-28",
  "sections": [
    {
      "repo_path": "/Users/jimnist/code/client/acme",
      "client_id": 123, "client_name": "Acme",
      "project_id": 234, "project_name": "Billing Portal",
      "task_id": 345, "task_name": "Development",
      "entries": [
        {
          "id": "2026-07-28-acme-billing-portal-development-001",
          "hours": 1.25, "billable": true, "approved": false, "needs_review": true,
          "notes": "Implemented webhook retry handling",
          "harvest_time_entry_id": null
        }
      ]
    }
  ]
}
```

A **Section** groups entries by `(repo_path, client, project, task)` and is a storage/display grouping only. **Approval is per entry** [ADR-0004]. Entry IDs are `YYYY-MM-DD-<client>-<project>-<task>-###`, the suffix incrementing within a Section. (Legacy files carried `approved` on the section; it is migrated onto the entries on load.)

## Mapping

`$JIMTIME_HOME/config/harvest-projects.json` maps a repo's canonical `git rev-parse --show-toplevel` to a Harvest client/project/default-task + default billable flag, plus reusable task aliases. One repo → one client/project. An unmapped repo or a non-repo cwd is a clear error. Multiple sections in a day arise only from task overrides.

## Rules

- **Billing:** store exact hours; no rounding on push.
- **Review/approve** operate over a date range *or* a single day (`--today`/`--week`/`--last-week`/`--date`/`--from`+`--to`).
- **Approval** is per entry [ADR-0004]. `approve` sweeps every unapproved entry in scope except `needs-review` ones (held) and `--except <id>`; `--include-needs-review` sweeps those too. `review --pending` lists what's outstanding first.
- **Import dedup:** `harvest_time_entry_id` lives on each entry; push skips entries that already have one.
- **Push safety:** `harvest push` is explicit only; pushes `approved && billable && !imported` entries (`Entry::is_pushable`); saves each id back immediately; fails loud, no silent partial success.
- **needs_review** is a structured flag on the entry (the View shows it); notes are not polluted with markers.

## Install

`cargo install --path .` puts a `jimtime` binary on `$PATH`. It runs in the caller's cwd, so `git rev-parse --show-toplevel` resolves the real working repo - no wrapper/alias hack.

## Skill

`/jimtime` - user-invoked only (`disable-model-invocation: true`), symlinked into `~/.claude/skills/`. Drives add/review/approve/push with human gates on approve and push. Source lives in this repo under `.claude/skills/jimtime/`.

## Build phases (all complete)

1. **Local logging:** `status`, `map`, `add`, `today`.
2. **Review/approval:** `review` (+`--pending`), `approve` (+`--except`/`--include-needs-review`), `unapprove`.
3. **Harvest:** `harvest dry-run`, `harvest push` (+ `projects`/`clients`/`tasks` lookups).
4. **Polish:** `report` markdown export, test suite, task aliases, date shortcuts, the `/jimtime` skill, install docs.
