# Docker and compose

`deploy/compose.yaml` runs Postgres with pgvector and `lampl serve` together. It was fixed and tested against the published `ghcr.io/moneymikemd/lamplighter:0.1.0` image while writing this page.

## What it starts

| Service | Image | Role |
| --- | --- | --- |
| `postgres` | `pgvector/pgvector:pg17` pinned by digest | Database with a health check; data in the `pgdata` volume |
| `init` | the lamplighter image, as root | One-shot: creates `/state/keys` with mode 600 and hands `/state` and the model cache to uid 10001, because named volumes start root-owned |
| `lampl` | `ghcr.io/moneymikemd/lamplighter:${LAMPLIGHTER_VERSION:-latest}` | `lampl serve --listen 0.0.0.0:7878`; port published on `127.0.0.1:7878` only |

Volumes: `pgdata` (database), `state` (keys file), `models` (embedding weights under `/home/lampl/.cache`).

## Walk-through

```sh
export LAMPLIGHTER_DB_PASSWORD=$(openssl rand -hex 16)
export LAMPLIGHTER_VERSION=0.1.0                     # optional; defaults to latest

# 1. create the first key before the first start (the server refuses an empty keys file)
docker compose -f deploy/compose.yaml run --rm lampl serve keys add laptop

# 2. download the embedding model into the models volume (about 1.1 GB, one time; it then says `not configured` and exits 1, which is expected)
docker compose -f deploy/compose.yaml run --rm lampl doctor --fetch-model

# 3. start
docker compose -f deploy/compose.yaml up -d
```

Step 1 prints `lampl_...` once; keep it. When the log shows `lampl serve ready` (the model takes about 25 seconds to load), check health and connect a client:

```sh
export LAMPL_API_KEY=lampl_...        # the key from step 1
curl -H "Authorization: Bearer $LAMPL_API_KEY" http://127.0.0.1:7878/health
lampl init --server http://127.0.0.1:7878
lampl doctor
```

Tested sequence: with the model downloaded, `/health` reported the embedder available, `lampl store` embedded two memories, `doctor` reported `memories: 3 (3 embedded)` and `degraded: false`, `link`
and `audit` worked, and `export` produced NDJSON with the embedding model id and digest in its header. Without step 2 the server starts but reports `degraded` and serves full-text recall only.

## Operating it

- Add a key without restarting: `docker compose -f deploy/compose.yaml run --rm lampl serve keys add NAME`; the running server notices within a second.
- Stop and keep data: `docker compose -f deploy/compose.yaml down`. Delete everything, memories included: add `-v`.
- Change the exposed address: edit the `ports` line. The server has no TLS; do not publish it beyond localhost without a reverse proxy.
- Upgrade: set `LAMPLIGHTER_VERSION` and `docker compose ... pull && up -d`.
- Resource use: the `lampl` container held about 2.5 GiB with the model loaded.

## Why the compose file changed

The version in 0.1.0 set `DATABASE_URL`, which the server does not read (it reads `LAMPL_DATABASE_URL`), had no keys file, published port 8080 on every host interface while the server listens on
127.0.0.1:7878 inside the container. Run as shipped, the container exited with `no database URL`. The file now sets `LAMPL_DATABASE_URL` and `LAMPL_KEYS_FILE`, passes `--listen 0.0.0.0:7878`,
publishes `127.0.0.1:7878` and adds the `init` service.

## Running the image without compose

`docker run ghcr.io/moneymikemd/lamplighter serve --listen 0.0.0.0:7878` needs `LAMPL_DATABASE_URL`, a mounted keys file readable only by uid 10001 (mode 600), and a writable model cache at
`/home/lampl/.cache`. Compose handles the permissions; doing it by hand is possible but fiddly.
