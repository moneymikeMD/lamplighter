# Embedded SQLite

An embedded store keeps memories in one SQLite file on your machine, with no server, no key and no database to run. It exists only when you ask for it. Internals and the known gaps against Postgres
are in [../local-mode.md](../local-mode.md); this page is the practical guide.

## Set up

```sh
lampl init --local --prepare
```

`init --local` creates `lampl.db` in the data directory and writes `config.toml` with `mode = "local"` and the database path. `--prepare` downloads the embedding model (Qwen3-Embedding-0.6B, about 1.1
GB, one time). Without it, `store` and `recall` fail with a model-missing message; pass `--no-embed` to use full text only.

Directories default to the platform config and data directories plus `lampl` (for example `~/Library/Application Support/lampl` on macOS). Override with `LAMPL_CONFIG_DIR` and `LAMPL_DATA_DIR`. The
model cache is separate, in the platform cache directory under `lamplighter/models`.

## Use

```sh
lampl store --type fix --project myapp --title "Run migrations before rollout" --body "..."
lampl recall "deploys hang at migrate" --project myapp
```

Only `store` and `recall` exist. `get`, `link`, `related`, `doctor`, `audit`, `export` and `import` print that they need a server and exit non-zero.

## Behaviour to know

- A deleted `lampl.db` is an error ("missing; run `lampl init --local`"), never a fresh empty store.
- `init` refuses to switch an existing config between local and server mode.
- The vector arm is an exact scan, not an index; fine at personal scale, untested at very large stores.
- SQLite stemming (porter) and ordering (`bm25`) differ from Postgres (`english` configuration, `ts_rank_cd`), and the relevance floor was calibrated on Postgres. Measured parity on real data is not
  done.
- The Claude Code plugin works in this mode: the hook calls `lampl recall --json`, and `lampl mcp` serves `store` and `recall`.

## When to leave

Choose a server when you want to share memory between machines or need links, `SUPERSEDES`, audit or export. Embedded memories cannot be exported or imported into a server today; re-store what you
need to keep.
