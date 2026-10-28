# The data home is its own git repo, committed and pushed automatically

ADR-0001 and ADR-0002 already made the store a diffable record meant to live in a private git repo, but committing was manual.
We want every change to be recorded and backed up without thinking about it, and a second machine to just work.
Git already gives history, a private remote, and auth the user has set up (SSH keys, credential helpers), so it is also the sync layer.
We rejected Google Drive/Dropbox as a data sync: a second copy that can drift, with no history and no merge.

## Decision

- **Sync is active only when `$JIMTIME_HOME` is the toplevel of its own git work tree** and `[git] auto_sync` is true (the default).
  If the data home sits inside some other repo, jimtime never commits, pulls or pushes there: that repo holds other people's or other projects' changes, and jimtime cannot know what is safe to touch.
  `jimtime data status` says which case applies.
- **Every command that writes** does: pull (`--rebase --autostash`), write, commit only the files it changed (with a descriptive message), push.
  A failed pull or push is a warning, not an error: the commit stays local and the next command retries.
  The exception is `invoice finalize`, which requires the pull to succeed (ADR-0008).
- **Day files merge semantically.**
  `.gitattributes` routes `entries/**/*.json` to a custom merge driver, `jimtime data merge-day`, registered in the repo's local git config by `data init` (and re-registered on every sync, since `.git/config` is not versioned).
  It does a 3-way merge of each Day: sections by their keys, entries by ID, fields one by one.
  Two machines that each added an entry to the same day both keep theirs.
  If both sides minted the same ID for different work, the side with no external link (no Harvest id, no invoice) is renumbered.
  A true conflict (the same field changed two ways) fails the merge, and nothing is guessed.
  jimtime then aborts the rebase, so the data home is never left half-rebased under a CLI that expects a clean tree, and the command stops with the exact `git pull --rebase` to run by hand.
- **Hand edits are the user's to record.**
  A command commits exactly the files it wrote, never a stray hand edit it did not make (autostash carries those across the pull).
  `jimtime data sync` commits them explicitly, after checking every day file and the config still parse, so a broken file never reaches another machine.
- `jimtime data init [--remote <url>]` makes the data home a repo (or clones one into it) and writes the `.gitattributes` and a `.gitignore` for `invoices/.drafts/`.

## Consequences

- With one machine, sync is just "commit and back up every change."
- With two, the common cases (different days, or the same day with different entries) merge without help.
- Offline work is fine except finalizing an invoice.
- The user moves their data out of a shared repo into a dedicated one (e.g. `jimtime-data`) to turn sync on; `git subtree split` preserves the history.
