# Per-day JSON files for the store, not SQLite

The store is one JSON file per day (`entries/YYYY/MM/YYYY-MM-DD.json`) containing that day's sections and entries.
We considered SQLite for transactions and queries but rejected it: a binary DB produces useless git diffs (defeating the committed, diffable billing record), adds a native dependency, and is overkill for one person's hours.

Per-day JSON renders 1:1 to the per-day markdown, rewrites atomically, stays human-inspectable, and needs only serde (already a dependency).
