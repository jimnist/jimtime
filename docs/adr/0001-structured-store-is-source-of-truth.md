# The structured JSON store is the only persisted artifact

The original spec made a daily markdown log the source of truth: the CLI would parse it, mutate it in place (flip `Approved`), and re-render it while preserving formatting.
We rejected that - round-tripping markdown (parse → mutate → re-render without mangling) is fragile and was the spec's own flagged hard part.

The source of truth is a structured store: one JSON file per day (`entries/YYYY/MM/YYYY-MM-DD.json`).
All mutations (`add`, `approve`, import-state) go through the CLI against the store.

We also dropped the persisted markdown view entirely.
JSON is already text and diffs cleanly in git, so it *is* the committed, human-inspectable billing record - a second markdown serialization would only duplicate the source of truth and have to be kept in sync on every write.
Human-readable rendering is done on demand and never persisted: `review`/`today` print to the terminal, Claude Code reads the JSON directly, and a future `report` command can export markdown when a shareable doc is actually wanted.

## Consequences

- One persisted artifact per day, not two. No renderer coupled to every mutation, no render-drift.
- Review and approval happen through the CLI (or Claude Code driving it), not by hand-editing files. The `.json` can be edited directly for corrections.
- Adding fields is a struct change, not a prose-schema change.
