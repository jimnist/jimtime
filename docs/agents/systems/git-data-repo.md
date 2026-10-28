# Git: the data repo

[ADR-0009](../../adr/0009-data-home-is-an-auto-synced-git-repo.md)

## What

`$JIMTIME_HOME` holds the config, the day files and the invoices.
When it is the toplevel of its own git repo, `datarepo::Sync` wraps every write: `begin` pulls (`--rebase --autostash`), writers call `datarepo::note_write(path)`, and `commit` commits exactly those paths and pushes.

## Auth

Whatever the user's git already uses (SSH keys, a credential helper).
jimtime adds no credentials of its own.

## Gotchas

- **Nested repos are left alone.**
  If `git rev-parse --show-toplevel` from the data home is not the data home itself, sync is off.
  That is deliberate: the enclosing repo is someone else's.
  `jimtime data status` prints a `git subtree split` recipe for moving the data out with its history.
- **The merge driver lives in `.git/config`**, which is not versioned, so `ensure_setup` re-registers it (by the absolute path of the running binary) on every sync, and a fresh clone gets it on its first write.
  `.gitattributes` (versioned) routes `entries/**/*.json` to it.
- **A true conflict aborts the rebase.**
  The CLI never leaves the data home mid-rebase; the error names the entry and field and the `git pull --rebase` to run by hand.
- **Renumbering.**
  Each machine mints entry ids `-001`, `-002`, ... on its own, so offline adds to the same day collide.
  The driver keeps both and renumbers the side with no Harvest id or invoice; its `jimtime:` stderr lines are passed through to the user.
- **Pull failures are warnings** except for `invoice finalize`, which must see the latest invoice numbers.
- **Hand edits** are not swept into a command's commit; `jimtime data sync` validates and commits them.
- **Invoice drafts** (`invoices/.drafts/`) are git-ignored via the `.gitignore` `ensure_setup` maintains.
