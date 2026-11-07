# lampl serve HTTP API

`lampl serve` is the only process that holds the database password and the embedder. Clients speak JSON over HTTP with a per-client API key. The server runs pending migrations on start and refuses to start if the database schema is newer than the build understands.

## Versioning

Every response, including errors and `/health`, carries `X-Lampl-Api-Version: <integer>`. The integer is `API_VERSION` in `crates/lampl/src/serve/mod.rs`. It is bumped by hand on any breaking change under `/v1` and has no relation to the crate semver, because breaking changes ship as `feat:` under the workspace version cap. Additive changes (new fields, new endpoints) do not bump it.

## Authentication

`Authorization: Bearer lampl_<64 hex>` on everything under `/v1`; a missing or unknown key is `401` with `WWW-Authenticate: Bearer`. `/health` is open so monitors need no key; it exposes counts only.

Keys live as `<client> <sha256-hex>` lines in the file named by `--keys-file` or `$LAMPL_KEYS_FILE` (`#` comments allowed). The server stores only hashes. `lampl keygen <client>` prints a new key (give it to the client, it is not recoverable) and the line to append. Deleting a line revokes that client alone; the file is re-read within a second of changing, no restart. A malformed edit is logged and the previous keys stay active. The server will not start with no keys file or an empty one.

## Configuration

| Flag | Environment | Default |
| --- | --- | --- |
| `--database-url` | `LAMPL_DATABASE_URL` | required |
| `--keys-file` | `LAMPL_KEYS_FILE` | required |
| `--listen` | | `127.0.0.1:7878` |
| `--embed-endpoint`, `--embed-endpoint-kind openai\|ollama`, `--embed-model` | `LAMPL_EMBED_API_KEY` (endpoint bearer) | in-process Qwen3-Embedding-0.6B |
| `--min-coverage` | | `0.95` |
| `--embed-interval-secs` | | `5` |

If the in-process model weights are missing the server still starts, reports `degraded`, and serves full-text recall.

## Endpoints

Errors are `{"error": "<message>"}` with the matching status.

### `GET /health`

```json
{
  "status": "ok",
  "degraded": false,
  "degraded_reasons": [],
  "api_version": 1,
  "schema_version": 1,
  "embed_mode": "in-process",
  "embed_model": "Qwen/Qwen3-Embedding-0.6B",
  "embedder_available": true,
  "embedder_error": null,
  "memory_count": 0,
  "embedded_count": 0,
  "vector_coverage": 1.0,
  "min_coverage": 0.95
}
```

`status` is `ok` only when `degraded` is false. `degraded` is true when the last embedder call failed (recall is then full-text only) or `vector_coverage` is below `min_coverage`. `vector_coverage` is rows embedded by the current model over all rows, and is `1.0` for an empty store, where `memory_count: 0` says so explicitly. `503` with `status: "unavailable"` when the database cannot be queried. Monitors should assert `degraded == false` and the coverage they expect, not just HTTP 200.

### `POST /v1/memories`

Body: `memory_type` (`fact`, `decision`, `problem`, `fix`, `procedure`, `gotcha`), `title`, `body`, `project`, optional `tags` (default `[]`), `importance` (0..1, default 0.5). Returns `201` with the memory once the row is written; the vector is added later by the background worker, so `embedding_model` is `null` at first.

### `GET /v1/memories/{id}`

The memory, including superseded ones. `404` if unknown.

### `GET /v1/memories?project=&limit=&after=`

Memories in id order (id is time-ordered), all of them including superseded. `limit` 1..=1000 (default 50). Returns `{"memories": [...], "next_after": "<id>"|null`}; pass `next_after` as `after` for the next page.

### `POST /v1/recall`

Body: `query`, optional `project`, `limit` (1..=100, default 10). Returns `{"mode": "hybrid"|"full-text", "hits": [{"memory", "score", "cosine", "coverage"}]}`. Hybrid full-text plus vector ranking with reciprocal-rank fusion and the relevance floor; superseded memories are excluded. `mode` is `full-text` when the embedder is unavailable, and rows not yet embedded are still found through full-text.

### `POST /v1/links`

Body: `from_id`, `to_id`, `link_type` (`SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS`), optional `strength` (0..1, default 1). `201` with the link; `400` for a self link or unknown type, `404` for an unknown memory.

### `GET /v1/memories/{id}/related`

`[{"direction": "outgoing"|"incoming", "link": {...}, "memory": {...}}]`, superseded memories included.

### `GET /v1/export?project=`

`application/x-ndjson`, streamed in pages: one `{"kind": "memory", ...}` line per memory in id order, then one `{"kind": "link", ...}` line per link. Vectors are not exported; they are recomputed.

### `POST /v1/import`

Body: the NDJSON produced by export (up to 256 MiB). Atomic: any bad line or a link to an unknown memory is `400` and nothing is written. Existing ids and links are skipped, so re-importing is safe. Returns `{"memories_imported", "memories_skipped", "links_imported", "links_skipped"}`. Imported rows are embedded by the background worker.

## Background embedding

A tokio task claims rows with no vector, or a vector from a different model digest, using `SELECT ... FOR UPDATE SKIP LOCKED` in batches of 16, embeds them with the configured embedder and writes the vectors. It sweeps every `--embed-interval-secs`, replacing memorygraph's hourly reindex timer. While idle it probes the embedder every 60 s (every sweep while unhealthy) so `/health` tracks reality.
