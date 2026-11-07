# Experiment: embedding inside Postgres with pgrx (LAMP-15)

Status: experiment only. This is not a supported mode; whether it becomes one is decided by the in-process vs pgrx showdown ticket on the grown gold set.

## What was built

- `crates/lampl-pg-ext`: a pgrx 0.19.3 extension. `shared_preload_libraries = 'lampl_pg_ext'` starts one background worker, `lampl embedder`. It loads `lampl-embed`'s `LocalEmbedder` (Qwen3-Embedding-0.6B, candle, CPU) once, polls
  `memories` every `lampl.poll_ms` (default 250) for rows with `embedding IS NULL` using `FOR UPDATE SKIP LOCKED`, embeds `title || E'\n' || body` in batches of `lampl.batch_size` (default 8), and writes `embedding`,
  `embedding_model` and `embedding_digest`. Client backends only INSERT; none of them touches the model. The S04 schema (`crates/lampl-store-pg/migrations/0001_init.sql`) is unchanged. The worker idles until that schema exists.
- `docker/pg-ext/`: Postgres 17 (ECR Public official image, pinned by digest) plus pgvector v0.8.7 built from source plus the extension. pgvector's own image is not published on ghcr.io (anonymous pull denied), so the
  base follows the rule's second choice. The weights are not baked in (about 1.1 GB); mount a Hugging Face cache at `/var/lib/lampl/models`. `docker/pg-ext/fetch-model.sh <dir>` creates it.
- The crate is excluded from the root workspace (`exclude` in the root `Cargo.toml`) because `pgrx-pg-sys` needs a `pg_config`, which workspace CI does not have. It has its own `Cargo.lock`, `publish = false`, and is not in
  `publish.yml`, `release-please-config.json` or the compose bundle.

Postgres version: 17. pgrx 0.19.3 supports 13 to 18 and pgvector 0.8.7 supports 13 to 18; 17 matches the store's CI service container. Only 17 was built and run.

## Reproduce

```bash
docker build -f docker/pg-ext/Dockerfile -t lampl-pg-ext:test .        # context is the repo root: the crate path-depends on crates/lampl-embed
docker/pg-ext/fetch-model.sh /tmp/lampl-models
docker run -d --name lampl-pgx -e POSTGRES_PASSWORD=test -p 54330:5432 -v /tmp/lampl-models:/var/lib/lampl/models:ro lampl-pg-ext:test
psql ... -f crates/lampl-store-pg/migrations/0001_init.sql
# INSERT a row, then SELECT vector_dims(embedding) FROM memories WHERE id = <new>   -> 1024
docker rm -f lampl-pgx
```

The ticket's `docker build ... docker/pg-ext` form cannot work: with that directory as the build context the Dockerfile cannot see `crates/`. Use the repo root as context.

## Results (2026-11-06, Docker Desktop on Apple silicon, linux/arm64, CPU only)

| Measure | Result |
| --- | --- |
| Image build | about 4 min cold (toolchain, cargo-pgrx, pgrx-pg-sys bindgen, candle); the package step alone took 44 s with a warm target cache |
| Image size | 472 MB, against 459 MB for the stock `postgres:17-bookworm` base: +13 MB (pgvector plus the extension) |
| Model load time | 8.5 s to 13.5 s per worker start (page cache cold vs warm); the worker restarts with the server |
| Insert to embedded, N | **N = 5 s.** First row after start 5.4 s; three warm single rows 1.6 s, 1.8 s, 2.4 s, measured with a `docker exec psql` poll, so a few hundred ms of that is client overhead plus the 250 ms poll interval |
| Throughput | 50 rows from 10 concurrent inserters were all embedded about 20 s after the first insert (about 0.4 s per row, batched) |
| `vector_dims(embedding)` | 1024; `embedding_model = Qwen/Qwen3-Embedding-0.6B`, `embedding_digest = 97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3` (the snapshot revision) |
| RAM, model worker | 2404 MiB RSS (2381 MiB anon): the weights are converted to f32, so the 986 MB bf16 file becomes about 2.4 GB resident |
| RAM per client backend | about 21 to 22 MiB RSS, 4 MiB anon, with 10 concurrent connections open and inserting |
| RAM, 10 connections | the worker stayed at 2404 MiB before, during and after; no backend loaded a model copy. Container cgroup: 2451 MiB idle, 2460 MiB during, 2452 MiB after |

The ten connections cost the process group roughly 40 MiB of anonymous memory in total. The dominant cost is the one worker's 2.4 GB, paid once per server whether or not any client is connected.

## Upkeep burden

- A pgrx extension is tied to a Postgres major: each supported major needs its own `cargo pgrx package` and image tag, and pgrx must track new majors (0.19.3 covers 13 to 18).
- The worker must be in `shared_preload_libraries`, so enabling or upgrading it needs a server restart; managed Postgres offerings generally will not allow it.
- Image upkeep: the base digest, the pgvector tag and the pgrx version are three pins to refresh.
- `pgrx-pg-sys` makes the build slow and heavy for CI, which is why the crate is kept out of the workspace build.
- `BackgroundWorker::transaction` requires an `UnwindSafe` closure, and candle types are not; the code wraps the embedder in `AssertUnwindSafe`. A panic inside candle aborts that transaction and restarts the worker after 5 s.
- Embedding failures `error!()` the worker, which reloads the model on restart (8 to 13 s); there is no per-row retry or dead-letter handling.
- The worker serves one database, set by `lampl.database` at server start.

## Not done

- No comparison against the in-process mode on the gold set (the showdown ticket).
- Only linux/arm64 and Postgres 17 were exercised; amd64 and other majors are untried.
- Embedding text is `title || E'\n' || body`; the store has no embedding-text convention yet, so vectors from the two modes are comparable only once one is fixed.
