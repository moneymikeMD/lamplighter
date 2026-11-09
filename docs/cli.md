# lampl client

The `lampl` client talks to `lampl serve` and fails closed: an unreachable server, a rejected key or an incompatible API version is a non-zero exit with a message, never an empty result. A client configured for a server never falls back to local mode.

## Configuration

| Setting | Source |
| --- | --- |
| Server URLs | `LAMPL_SERVER` (comma-separated, tried in order) or `lampl init --server URL [--server FALLBACK]` |
| API key | `LAMPL_API_KEY`, or `api_key_file` in `config.toml` (`lampl init --server URL --api-key-file PATH`) |
| Config and data dirs | `LAMPL_CONFIG_DIR`, `LAMPL_DATA_DIR` |

`LAMPL_SERVER` wins over the config file, so a machine with no config can still use a server. Keys are never printed. The first reachable URL is used; once a server answers, errors from it (401, 4xx, 5xx) are reported rather than failed over. Every response must carry `X-Lampl-Api-Version` equal to the client's version, otherwise the command stops naming both numbers.

## Commands

| Command | What it does |
| --- | --- |
| `store --title T --body B [--type fact] [--tags a,b] [--project P] [--importance 0.5]` | Store a memory; the server embeds it in the background |
| `recall QUERY` or `recall --query QUERY [--project P] [--limit N] [--json] [--embed-timeout-ms MS]` | One JSON hit per line; warns on stderr when the server could only do full-text. `--json` prints a single `{"hits": [...], "warning": null}` object instead, so an empty recall is `{"hits": []}` rather than no output. `--embed-timeout-ms` (or `LAMPL_EMBED_TIMEOUT_MS`) abandons embedding the query after that long and recalls by full text with a warning |
| `get ID` / `related ID` | Read a memory, or its links in both directions |
| `link FROM TO TYPE [--strength 1.0]` | `SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS` |
| `delete ID` | Permanently remove a memory and every link to or from it; prints `{"id", "links_removed"}`. Unknown id: non-zero exit, `not found`. See below |
| `doctor [--json] [--fetch-model]` | Server URL, store host, schema version, memory count, embed mode, vector coverage, degraded flag; exits non-zero on any problem. `--fetch-model` is the only place the embedding model is downloaded |
| `audit [--json] [--stale-days 365] [--project P]` | Read-only: duplicate titles and bodies, stale and untagged memories; superseded memories are excluded |
| `export [-o FILE] [--project P] [--no-vectors]` | NDJSON: a header (format, API version, embedding model id and digest), every memory with its vector, every link, then an end marker with counts. Nothing is written if the stream breaks |
| `import FILE` / `import --from-postgres [URL]` | See below |

Memory and link types are validated client-side and the valid set is printed on error. In local mode only `store`, `recall` and `delete` exist; the others say so and exit non-zero.

## Delete

`lampl delete ID` is for a memory that should never have been stored: a test row, an import duplicate, a leaked secret. It works in server mode (`DELETE /v1/memories/{id}`, see `api.md`)
and in local mode. The memory, its vector and full-text entry, and every link to or from it are removed in one transaction; the memories at the other end of those links stay. There is no
confirmation prompt and no undo, so `lampl export` first if in doubt. A second delete of the same id exits non-zero with `not found`.

The server logs each deletion with the id and the calling key's name, never the text. Local mode has no server log; the command's output is the only record.

To retire a memory that was true once and is now stale, do not delete it: `lampl link NEW OLD SUPERSEDES` keeps the old one reachable by id and drops it from recall and audit.

## Import

`import` detects the file: a lamplighter export, or a memorygraph JSON export (for example `~/.memorygraph/export-final-2026-09-28.json`). `--from-postgres` reads memorygraph's tables directly (URL from the flag or `LAMPL_FROM_POSTGRES_URL`, which keeps the password out of argv); this is the only place the client talks to a database.

- Lamplighter exports carry vectors, so no embedder is needed to restore. The import refuses when the export's model id or digest differs from the server's; `--reembed` drops the vectors and lets the server re-embed.
- Truncated exports (no end marker, or counts that disagree) are refused.
- memorygraph imports always re-embed. Mapping and what is dropped: `migrating-from-memorygraph.md`.
- Idempotent: ids and link endpoints are preserved or remapped deterministically and the server skips existing rows, so a second run adds nothing. `--dry-run` converts and checks without uploading.

## Restore is tested

`crates/lampl/tests/client_commands.rs` populates a server, exports, imports into a fresh database and asserts that recall returns identical results with no re-embedding. Run it with `LAMPL_TEST_DATABASE_URL` pointing at a pgvector Postgres superuser.
