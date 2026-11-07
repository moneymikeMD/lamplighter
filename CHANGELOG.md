# Changelog

## Unreleased

- `lampl-core`: lean typed memory model (`fact`, `decision`, `problem`, `fix`, `procedure`, `gotcha`), typed links (`SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS`), reciprocal-rank fusion and relevance floor.
- `lampl-store-pg`: Postgres and pgvector store with supersession, hybrid recall and a versioned, expand-contract schema.
- `lampl serve`: versioned `/v1` HTTP API (store, recall, get, link, related, list, export, import) with per-client API keys, background embedding, migrations on start and an honest `/health` (`lampl serve keys add|list|revoke`, open `/livez`). See `docs/api.md`.
- `lampl` client: `store`, `recall`, `get`, `link`, `related`, `doctor`, `audit`, `export` and `import` against `lampl serve`, failing closed on an unreachable server, a rejected key or an API version mismatch; import reads lamplighter exports (vectors included) and memorygraph exports. See `docs/cli.md` and `docs/migrating-from-memorygraph.md`.
