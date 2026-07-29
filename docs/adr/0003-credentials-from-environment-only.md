# Credentials come only from environment variables

Harvest credentials (`HARVEST_ACCESS_TOKEN`, `HARVEST_ACCOUNT_ID`, optional `HARVEST_USER_AGENT`) are read only from the environment.
The app never reads them from a config or `.env` file and never writes them to disk; it fails loudly, naming the missing variable, when they are absent.

We rejected dotenv / config-file support for secrets: an on-disk secret file is a commit-and-sync leak hazard, and this personal tool has a single credential set that belongs in the user's dotfiles.
This is distinct from non-secret config: `JIMTIME_HOME` is also an environment variable, and the repo→Harvest mapping is a committed, non-secret JSON file under `$JIMTIME_HOME/config/`.
