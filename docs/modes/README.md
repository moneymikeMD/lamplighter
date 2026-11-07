# Choosing a run mode

lamplighter has one binary and four ways to run it. A client picks its mode once with `lampl init`; no client ever falls back from one mode to another.

| | [Embedded SQLite](embedded-sqlite.md) | [`lampl serve` + Postgres](serve-postgres.md) | [External Postgres](external-postgres.md) | [Docker / compose](docker-compose.md) |
| --- | --- | --- | --- | --- |
| Setup | `lampl init --local --prepare` | Run Postgres with pgvector, a keys file and `lampl serve` | Same, against your database | `docker compose up -d` |
| Who shares the memory | One machine | Every client with a key | Every client with a key | Every client with a key |
| Store | SQLite, FTS5, sqlite-vec | Postgres with pgvector, `halfvec` and HNSW | Postgres with pgvector | Postgres 17 with pgvector in a container |
| Commands | `store`, `recall` | All | All | All |
| Embedder | In-process, on the client | On the server (in-process or HTTP endpoint) | On the server | On the server |
| Auth | None, local file | Per-client API keys | Per-client API keys | Per-client API keys |
| Needs a network | Only for the one-time model download | Client to server | Client to server | Client to server |
| Calibrated and measured | Fixtures only; not calibrated on real data | Yes, [ranking.md](../ranking.md) | Yes | Yes |

## Decision guide

- One person, one machine, and you only need store and recall: **embedded**.
- Two machines, a team, links, audit, export or import: **a server**. Pick compose to get one running in minutes, `lampl serve` directly if you run services yourself, and external Postgres if a
  database already exists.
- You cannot change mode in place. `lampl init` refuses to switch an existing config; move memories by exporting from a server and importing into another. Embedded stores have no export.

## Two rules that hold everywhere

1. **Fail closed.** A client configured for a server that is unreachable, rejects its key or speaks another API version exits non-zero. It never creates a local store or returns an empty list.
2. **No implicit downloads.** The embedding model is fetched only by `lampl init --local --prepare` and `lampl doctor --fetch-model`.
