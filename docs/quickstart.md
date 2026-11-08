# Quickstart

Two paths, both without an LLM key. The commands and outputs below were run against 0.1.0 (a local build for embedded mode; the published image for the server).

## Embedded, on one machine

```sh
lampl init --local --prepare
```

This creates `lampl.db`, writes `config.toml` with `mode = "local"`, and downloads Qwen3-Embedding-0.6B (about 1.1 GB, once). Without `--prepare` the store is created but `store` and `recall` stop
with `embedding model not downloaded` unless you pass `--no-embed`.

```sh
lampl store --type decision --project demo --tags auth \
  --title "Use Postgres for the session store" \
  --body "Chose Postgres over Redis: sessions must survive restarts and we already run Postgres. Revisit if write volume passes 5k/s."
lampl store --type gotcha --project demo --tags auth \
  --title "Redis eviction drops sessions" \
  --body "maxmemory-policy allkeys-lru silently evicted live sessions in staging."
```

Each prints the stored memory as JSON. Embedding one memory took about 4.5 seconds in a release build on an Apple M-series laptop, most of it loading the model.

```sh
lampl recall "why did we pick postgres over redis for sessions" --project demo
lampl recall "sessions getting evicted from redis" --project demo
lampl recall "kubernetes ingress timeout" --project demo
```

Measured scores on this two-memory store (the best hit first; with only two memories the second one often clears the floor too):

| Query | Hits (cosine / coverage) |
| --- | --- |
| why did we pick postgres over redis for sessions | the decision 0.838 / 0.371, then the gotcha 0.597 / 0.128 |
| sessions getting evicted from redis | the gotcha 0.783 / 0.371, then the decision 0.651 / 0.128 |
| kubernetes ingress timeout | none |

The third prints nothing: no candidate reached cosine 0.52 or coverage 0.35. Use `--json` to get `{"hits":[],"warning":null}` instead of silence. A short, vague query such as `where do we keep
sessions` returned both memories on this tiny store at low scores (cosine about 0.61 to 0.64, coverage 0.09), because with two memories the inverse-document-frequency weights are near zero and the cosine floor
is the only filter; phrase recall queries the way you would phrase a question about a past decision.

Embedded mode supports only `store` and `recall`. For links, audit and export, use a server.

## With a server

The fastest server is the compose file, which also creates the keys file the server demands. See [modes/docker-compose.md](modes/docker-compose.md) for the full walk-through, then point a client at
it:

```sh
export LAMPL_SERVER=http://127.0.0.1:7878
export LAMPL_API_KEY=lampl_...           # printed once by `lampl serve keys add`
lampl init --server "$LAMPL_SERVER"
lampl doctor
```

`doctor` prints the server, store, schema version, memory count, embed mode, vector coverage and a `degraded` flag; it exits non-zero on any problem. Then store, link and audit:

```sh
OLD=$(lampl store --type decision --project demo --title "Use Redis for sessions" --body "..." | jq -r .id)
NEW=$(lampl store --type decision --project demo --title "Use Postgres for sessions" --body "Replaces $OLD." | jq -r .id)
lampl link "$NEW" "$OLD" SUPERSEDES
lampl related "$NEW"
lampl audit
```

After the `SUPERSEDES` link, `audit` reported `1 superseded excluded` and recall stopped returning the old memory.

## Connect Claude Code

```sh
claude plugin marketplace add moneymikeMD/moneymike-plugins
claude plugin install lamplighter@moneymike-plugins
```

Start a session in a project and the hook injects matching memories. See [plugin.md](plugin.md).
