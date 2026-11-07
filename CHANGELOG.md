# Changelog

## Unreleased

- `lampl-core`: lean typed memory model (`fact`, `decision`, `problem`, `fix`, `procedure`, `gotcha`), typed links (`SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS`), reciprocal-rank fusion and relevance floor.
- `lampl-store-pg`: Postgres and pgvector store with supersession, hybrid recall and a versioned, expand-contract schema.
- `lampl serve`: versioned `/v1` HTTP API (store, recall, get, link, related, list, export, import) with per-client API keys, background embedding, migrations on start and an honest `/health` (`lampl keygen` creates keys). See `docs/api.md`.
