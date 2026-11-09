# Changelog

## [0.4.1](https://github.com/moneymikeMD/lamplighter/compare/v0.4.0...v0.4.1) (2026-11-09)


### Bug Fixes

* **release:** attach the host job's own dist-manifest.json to the release (LAMP-33) ([#48](https://github.com/moneymikeMD/lamplighter/issues/48)) ([1dfe854](https://github.com/moneymikeMD/lamplighter/commit/1dfe854ebccf8278c2ef62b423ff611980584259))
* **release:** create the lamplighter--v tag before the Cargo.lock sync, and parse the PR JSON in the script (LAMP-32) ([#47](https://github.com/moneymikeMD/lamplighter/issues/47)) ([90b96d0](https://github.com/moneymikeMD/lamplighter/commit/90b96d04ebbda4f7813f468b6860f0ccba3d588c))

## [0.4.0](https://github.com/moneymikeMD/lamplighter/compare/v0.3.0...v0.4.0) (2026-11-09)


### Features

* **cli:** lampl delete removes a mistaken memory and its links (LAMP-30) ([#41](https://github.com/moneymikeMD/lamplighter/issues/41)) ([b910a80](https://github.com/moneymikeMD/lamplighter/commit/b910a8020aa449f31ff2b948a32f4bbc5296f4b7))
* **serve:** name rows that fail to embed and retry them on request (LAMP-28) ([#42](https://github.com/moneymikeMD/lamplighter/issues/42)) ([bac48c9](https://github.com/moneymikeMD/lamplighter/commit/bac48c92616be6a21384bc9ea8da7b5fffe6edf0))


### Bug Fixes

* **embed:** endpoint embedder cuts input to the in-process 512-token window (LAMP-25) ([#43](https://github.com/moneymikeMD/lamplighter/issues/43)) ([31fc831](https://github.com/moneymikeMD/lamplighter/commit/31fc831207b39e6af7e0864cc6b46eed8be3aa19))
* **import:** map memorygraph 0.13 link types, REPLACES to SUPERSEDES, count unmapped types (LAMP-26) ([#39](https://github.com/moneymikeMD/lamplighter/issues/39)) ([1d6183e](https://github.com/moneymikeMD/lamplighter/commit/1d6183ecc279c68711355beae86cc23bd3ca60a6))
* **local:** embed title, a blank line, then body, as serve does (LAMP-31) ([#45](https://github.com/moneymikeMD/lamplighter/issues/45)) ([620c05e](https://github.com/moneymikeMD/lamplighter/commit/620c05e05c82521fb56170c2e985783aaa7b0cd6))
* **serve:** log to stderr, ANSI only on a terminal and without NO_COLOR (LAMP-27) ([#38](https://github.com/moneymikeMD/lamplighter/issues/38)) ([2392763](https://github.com/moneymikeMD/lamplighter/commit/2392763b54e8c9bc1857cc180b37fd17eca6b9e5))
* **serve:** refuse an empty or anonymous database user and document Unix-socket URLs (LAMP-29) ([#40](https://github.com/moneymikeMD/lamplighter/issues/40)) ([fcaf34e](https://github.com/moneymikeMD/lamplighter/commit/fcaf34e1e5a52f4fa0609abb8b091dc03ed2096d))

## [0.3.0](https://github.com/moneymikeMD/lamplighter/compare/v0.2.1...v0.3.0) (2026-11-08)


### Features

* **core:** default relevance floor uses capped:3 coverage at 0.52/0.52 (LAMP-22) ([#36](https://github.com/moneymikeMD/lamplighter/issues/36)) ([434ca4c](https://github.com/moneymikeMD/lamplighter/commit/434ca4c8184d0af4cf63da513f545b3c449b6609))
* **serve:** bench-only normalised floor coverage, LAMP-22 grid ([#35](https://github.com/moneymikeMD/lamplighter/issues/35)) ([ecbf774](https://github.com/moneymikeMD/lamplighter/commit/ecbf774ee68a1b5de322b89f28dab3a67b110aec))


### Bug Fixes

* **plugin:** breaker notice stays in the session that failed; new sessions probe to close it ([#33](https://github.com/moneymikeMD/lamplighter/issues/33)) ([fa74db9](https://github.com/moneymikeMD/lamplighter/commit/fa74db9dc9623e261d5a52b3fb21c424c6e70366))

## [0.2.1](https://github.com/moneymikeMD/lamplighter/compare/v0.2.0...v0.2.1) (2026-11-08)


### Bug Fixes

* **release:** keep Cargo.lock in step with the release version ([#31](https://github.com/moneymikeMD/lamplighter/issues/31)) ([3592270](https://github.com/moneymikeMD/lamplighter/commit/3592270aa22a6b6eedd22be8d86dbdd2666603fd))

## [0.2.0](https://github.com/moneymikeMD/lamplighter/compare/v0.1.0...v0.2.0) (2026-11-08)


### Features

* **bench,embed:** checkpointed lampl-bench runs and an opt-in Metal embedder ([#22](https://github.com/moneymikeMD/lamplighter/issues/22)) ([fd54ba8](https://github.com/moneymikeMD/lamplighter/commit/fd54ba8e93852efa1048fcfc82276ffb71b23bfa))
* **core:** lower the default relevance floor coverage from 0.24 to 0.18 (LAMP-21) ([#27](https://github.com/moneymikeMD/lamplighter/issues/27)) ([826e26c](https://github.com/moneymikeMD/lamplighter/commit/826e26c13fa4238e224e5c7579933ec02e748a80))


### Bug Fixes

* **core:** case-invariant recall, Qwen3 query instruction, cosine+coverage ordering (LAMP-23) ([#29](https://github.com/moneymikeMD/lamplighter/issues/29)) ([940a310](https://github.com/moneymikeMD/lamplighter/commit/940a31083f13bb20201a253c508401311c645537))
* **core:** stop words no longer count against coverage; default floor 0.52/0.35 (LAMP-23) ([#30](https://github.com/moneymikeMD/lamplighter/issues/30)) ([53f37b1](https://github.com/moneymikeMD/lamplighter/commit/53f37b1b3ffc5ae5c835230a9c80892b4aabc2a5))

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
