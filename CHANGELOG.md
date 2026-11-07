# Changelog

## 0.1.0 (2026-11-07)


### Features

* Cargo workspace scaffold builds, lints and tests on CI (LAMP-4) ([47fe8e1](https://github.com/moneymikeMD/lamplighter/commit/47fe8e1b47d834180d3821d4f8aa21b8b0fdf620))
* Claude Code plugin pushes, pulls and teaches; lampl recall --json (LAMP-10) ([#14](https://github.com/moneymikeMD/lamplighter/issues/14)) ([65a1ad7](https://github.com/moneymikeMD/lamplighter/commit/65a1ad7516c49e9871ef23b1df1ad19a2bb25ff9))
* embedded SQLite local mode exists only when chosen (LAMP-11) ([#7](https://github.com/moneymikeMD/lamplighter/issues/7)) ([c105f4d](https://github.com/moneymikeMD/lamplighter/commit/c105f4d9f459865a948d2de766c8c6e3b672ceb1))
* experimental pgrx extension embeds memories inside Postgres (LAMP-15) ([#10](https://github.com/moneymikeMD/lamplighter/issues/10)) ([c05455f](https://github.com/moneymikeMD/lamplighter/commit/c05455fc815efa42b80c84b3048c3543874e4ecb))
* lampl client stores, recalls, diagnoses, exports and imports, failing closed (LAMP-8) ([#12](https://github.com/moneymikeMD/lamplighter/issues/12)) ([18d5965](https://github.com/moneymikeMD/lamplighter/commit/18d59659c7ce6f2fcff8c5ea5cb15c0d3ccfc165))
* lampl mcp advertises the memory schema over MCP stdio (LAMP-9) ([#13](https://github.com/moneymikeMD/lamplighter/issues/13)) ([28d7b9c](https://github.com/moneymikeMD/lamplighter/commit/28d7b9cdb7b7d93f0b684ec1832feaebeae0fb7a))
* lampl serve with versioned HTTP API, API keys, background embedding and honest health (LAMP-7) ([#8](https://github.com/moneymikeMD/lamplighter/issues/8)) ([0a4ebda](https://github.com/moneymikeMD/lamplighter/commit/0a4ebda9a922eb2049c4be03d3b32037f9924408))
* lampl-embed embeds Qwen3-Embedding-0.6B in-process on CPU, with bench-embed (LAMP-5) ([08d0535](https://github.com/moneymikeMD/lamplighter/commit/08d053528e0110d0ef16bf552c9070cdec4f0213))
* Postgres store with lean typed model, supersession and hybrid recall (LAMP-6) ([#5](https://github.com/moneymikeMD/lamplighter/issues/5)) ([47878e0](https://github.com/moneymikeMD/lamplighter/commit/47878e0b2e3cc696a4fe8680651bece87819560d))
* public LongMemEval retrieval benchmark ships in the repo (LAMP-14) ([#15](https://github.com/moneymikeMD/lamplighter/issues/15)) ([76c8539](https://github.com/moneymikeMD/lamplighter/commit/76c8539629af4a1ef409af85cc5dcdbf21f5a209))
* recall floor calibrated on Qwen3 vectors, cosine 0.70 and coverage 0.24 (LAMP-19) ([#17](https://github.com/moneymikeMD/lamplighter/issues/17)) ([4279710](https://github.com/moneymikeMD/lamplighter/commit/4279710a800eca874c47d6c6a7eac50f738f5d8b))
* tagged releases ship binaries, crates and an image (LAMP-12) ([#6](https://github.com/moneymikeMD/lamplighter/issues/6)) ([d7f4c1e](https://github.com/moneymikeMD/lamplighter/commit/d7f4c1ea1f861834d6dc718937b206c9ca8ac8c4))


### Bug Fixes

* serve fails closed on keys-file trouble, leases embedding claims, gates vector recall by digest (LAMP-20) ([#11](https://github.com/moneymikeMD/lamplighter/issues/11)) ([12a938e](https://github.com/moneymikeMD/lamplighter/commit/12a938e9bfe725e54394df7575235120f3c683ef))

## Changelog

## Unreleased

- `lampl-core`: lean typed memory model (`fact`, `decision`, `problem`, `fix`, `procedure`, `gotcha`), typed links (`SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS`), reciprocal-rank fusion and relevance floor.
- `lampl-store-pg`: Postgres and pgvector store with supersession, hybrid recall and a versioned, expand-contract schema.
- `lampl serve`: versioned `/v1` HTTP API (store, recall, get, link, related, list, export, import) with per-client API keys, background embedding, migrations on start and an honest `/health` (`lampl serve keys add|list|revoke`, open `/livez`). See `docs/api.md`.
- `lampl` client: `store`, `recall`, `get`, `link`, `related`, `doctor`, `audit`, `export` and `import` against `lampl serve`, failing closed on an unreachable server, a rejected key or an API version mismatch; import reads lamplighter exports (vectors included) and memorygraph exports. See `docs/cli.md` and `docs/migrating-from-memorygraph.md`.
- Claude Code plugin (`plugin/`, install id `lamplighter@moneymike-plugins`): push hooks for session start, each prompt and failed Bash calls, `lampl mcp` registered from PATH, and the `lamplighter` usage skill. `lampl recall --json` prints one `{hits, warning}` object, so an empty recall is `{"hits": []}`. Hook logic is ported from night-watchman's MIT-licensed `memory-push.py`.
