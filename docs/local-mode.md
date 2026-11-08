# Local mode

Local mode is an embedded SQLite store for people who want memory without running Postgres. It exists **only** when you ask for it with `lampl init --local`.

## Rules

- `lampl init --local` creates the database (`lampl.db` in the data dir) and writes `config.toml` with `mode = "local"` and the database path.
- A client with no config exits non-zero with ``not configured: run `lampl init` `` and creates nothing.
- A client configured with `lampl init --server <URL>` never falls back to local mode. If the server is down it exits non-zero ("not falling back to local mode") and creates no file.
- If a local database file is deleted, commands fail ("missing; run `lampl init --local`"); they do not open a fresh empty store.
- `init` refuses to switch an existing config between modes.

Directories come from `LAMPL_CONFIG_DIR` and `LAMPL_DATA_DIR`, defaulting to the platform config and data dirs plus `lampl`.

## Embedding model

Embedding is in-process (`lampl-embed`, Qwen3-Embedding-0.6B on CPU; build with `--features metal` on Apple Silicon to run it on the GPU, falling back to CPU when no Metal device exists, and set `LAMPL_EMBED_DEVICE=cpu` to force CPU). The weights are not bundled and are never downloaded implicitly. Download them once:

```bash
lampl init --local --prepare
```

`lampl store` and `lampl recall` fail with a "model weights not found" error until then; `--no-embed` skips embedding and uses full text only.

## Storage

`lampl-store-sqlite` uses `rusqlite` (bundled SQLite), an FTS5 table (`porter unicode61` tokenizer) ranked by `bm25`, and a `sqlite-vec` `vec0` table (`float[1024]`, cosine). `sqlite-vec` is pinned to
exactly 0.1.9 (pre-v1; the 0.1.10 builds are alpha). Fusion, the relevance floor and IDF coverage reuse `lampl-core` (`rrf_fuse`, `passes_floor`, `floor_coverage`), including the default `capped:3`
floor coverage, and the memory model, supersession and link semantics match the Postgres store.

The vector arm is an exact scan over active memories, not an index, which is fine at local-mode sizes.

## Postgres parity

The same hand-built fixtures (fusion order, supersession exclusion, floor, link round trip) pass against both stores. Measured parity on real data is **not done**; known differences:

- Stemming: SQLite uses porter, Postgres uses the `english` configuration, so some inflections match in one and not the other.
- Full-text ordering: `bm25` versus `ts_rank_cd`.
- Vectors: SQLite stores `float32` and scans exactly; Postgres stores `halfvec` behind an HNSW index.
- Coverage document frequency is counted with FTS5 matches versus tsvector matches.

Closing this gap needs the gold set from the calibration ticket (see `ranking.md`).

## Dependency note

`rusqlite` is 0.39.0, not the 0.41.1 named in the library survey: `sqlx` 0.9.0 declares an optional `sqlx-sqlite` that needs `libsqlite3-sys <0.38`, and Cargo allows one crate linking `sqlite3` in the graph even when
that feature is off, so rusqlite 0.41 (`libsqlite3-sys 0.39`) cannot resolve beside it. Revisit when sqlx releases a version that allows newer `libsqlite3-sys`.
