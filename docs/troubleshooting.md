# Troubleshooting and FAQ

Messages below are quoted from the code. Start with `lampl doctor`: it names the server, store, schema version, memory count, embed mode, vector coverage and `degraded`, and exits non-zero on a
problem.

## Symptoms

| Symptom | Cause | Fix |
| --- | --- | --- |
| ``not configured: run `lampl init` `` | No `config.toml` yet; nothing is created implicitly | `lampl init --local --prepare` or `lampl init --server URL` |
| `embedding model not downloaded; run lampl init --local --prepare once, or pass --no-embed` | Weights are fetched only on request | Run the named command, or `lampl doctor --fetch-model` |
| `lampl: server URL is unreachable ...; not falling back to local mode` | The server is down, the URL is wrong, or it is still loading the model (about 25 s after start) | Check `curl URL/livez`; wait; the client never falls back by design |
| `401` from a server | Missing, unknown or revoked key | Set `LAMPL_API_KEY` or `api_key_file`; create one with `lampl serve keys add NAME` |
| Client stops and names two API version numbers | Client and server speak different `X-Lampl-Api-Version` | Upgrade the older side |
| `no database URL: set LAMPL_DATABASE_URL or pass --database-url` | `lampl serve` read no URL. `DATABASE_URL` is not read | Set `LAMPL_DATABASE_URL` |
| `no keys file: set LAMPL_KEYS_FILE ...` or `keys file ... holds no keys; refusing to serve without authentication` | The server refuses to run unauthenticated | `lampl serve keys add NAME` first |
| `keys file ... is readable by group or others (mode 644); run chmod 600` | Permissions too open | `chmod 600` the file |
| `database schema version N is newer than this build understands` | An older binary against a newer database | Upgrade `lampl` |
| `/health` says `degraded`, `embedder unavailable` | Model weights missing or the endpoint is down; recall is full-text only | `lampl doctor --fetch-model` on the server host, then restart `lampl serve`; check `--embed-endpoint` |
| `/health` says degraded for vector coverage | Fewer than 95% of memories have vectors, for example right after a large import | Wait for the background worker; lower `--min-coverage` only if you accept it |
| Recall prints nothing | Nothing cleared the floor (cosine 0.52 or coverage 0.35) | Rephrase as a question about the decision; try `--json`; confirm the memory exists with `get`; very small stores give weak IDF coverage |
| A memory I stored is missing from vector results | Not embedded yet (server mode embeds in the background) | It is found by full text at once; wait a few seconds |
| `lampl audit` / `link` / `export` says it needs a server | Embedded mode has `store` and `recall` only | Use a server ([modes](modes/README.md)) |
| ``missing; run `lampl init --local` `` | The embedded database file was deleted | Restore it or re-init; lampl will not open an empty store silently |
| Plugin says `lampl` missing | Not on the `PATH` Claude Code sees | Install it ([install.md](install.md)); restart Claude Code |
| Hook prints a timeout, or memories go quiet for up to five minutes | Its circuit breaker opened after a failure; the next new session probes and closes it once recall works | `LAMPL_PUSH_BREAKER`, or fix the cause named in the one-line message; see [plugin.md](plugin.md) |
| `docker compose` exits with `no database URL` | You are running the 0.1.0 `deploy/compose.yaml`, which set the wrong variable | Use the fixed file ([modes/docker-compose.md](modes/docker-compose.md)) |

## FAQ

**Do I need an API key or an LLM?** No. Storing and recalling make no model calls; embedding runs in the process on CPU.

**Why did recall return nothing?** That is the relevance floor doing its job. On a private 138-query set it returned nothing for 9 of 19 queries that had no relevant memory without costing recall
([ranking.md](ranking.md)). The `lampl` binary has no flag or variable for the floor; library users can set `Floor` on the store.

**Is anything sent to a third party?** No telemetry. The embedding model is downloaded from Hugging Face once, on request. See the README's Telemetry section.

**Can I use my own embedding model?** Through `--embed-endpoint` if it returns 1024-dimension vectors; the floor is calibrated only for Qwen3-Embedding-0.6B ([embeddings.md](embeddings.md)).

**Can two machines share memories without a server?** No. Embedded stores are per machine. Run a server ([modes](modes/README.md)).

**Where are my memories?** Embedded: `lampl.db` in the data directory. Server: the Postgres database named by `LAMPL_DATABASE_URL`.

**How do I back up?** `lampl export -o backup.ndjson` (server mode) and normal Postgres backups. Restore with `lampl import backup.ndjson`; it is idempotent.

**How do I delete a memory?** There is no delete command in 0.1.0. Retire it with a `SUPERSEDES` link, which removes it from recall; delete rows in SQL if you must.

**Does it work without Claude Code?** Yes. The CLI, HTTP API and MCP server stand alone; the plugin is one client.

**Where do I report a bug?** [GitHub issues](https://github.com/moneymikeMD/lamplighter/issues). Include `lampl --version` and `lampl doctor --json` with secrets removed.
