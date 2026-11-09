# `lampl serve` on Postgres

`lampl serve` is the one process that holds the database password and the embedder. Clients speak JSON over HTTP with a per-client API key. Endpoints are in [../api.md](../api.md).

## Requirements

- Postgres with the `vector` (pgvector) extension available. The stack was run on `pgvector/pgvector:pg17`. The schema uses `halfvec` and HNSW, so pgvector 0.7 or newer is expected (UNVERIFIED lower
  bound).
- About 2.5 GiB of memory for the server with the in-process model loaded (measured in a container).
- A keys file with at least one key. The server will not start without one.

## Steps

1. Start Postgres with pgvector. For a scratch database:

   ```sh
   docker run -d --name lampl-pg -e POSTGRES_PASSWORD=change-me -e POSTGRES_DB=lamplighter -p 127.0.0.1:5432:5432 pgvector/pgvector:pg17
   ```

2. Create a key. The file is created owner-only; the key is printed once and only its SHA-256 is stored.

   ```sh
   export LAMPL_KEYS_FILE=$HOME/.config/lampl/keys
   mkdir -p "$(dirname "$LAMPL_KEYS_FILE")"
   lampl serve keys add laptop
   ```

3. Download the model on the server host, then start the server. Use the environment for the URL so the password stays out of argv.

   ```sh
   lampl doctor --fetch-model            # downloads the model into this host's cache; then says `not configured` and exits 1, which is expected here
   export LAMPL_DATABASE_URL=postgres://postgres:change-me@127.0.0.1:5432/lamplighter
   lampl serve --listen 127.0.0.1:7878
   ```

The server runs pending migrations, creates the `vector` extension if missing, and logs `lampl serve ready`. If the model is missing it still starts, reports `degraded` and answers full-text only.

### Local Postgres over a Unix socket

Put the socket directory, URL-encoded, in the authority, and always name the role:

```sh
export LAMPL_DATABASE_URL="postgresql://$(id -un)@%2Ftmp/lampl"                      # Homebrew: socket in /tmp
export LAMPL_DATABASE_URL="postgresql://$(id -un)@%2Fvar%2Frun%2Fpostgresql/lampl"   # Debian, Ubuntu
```

Under launchd or systemd, `$(id -un)` is not evaluated, so write the literal role name into the plist or unit. Two other forms fail:

| URL | Result |
| --- | --- |
| `postgresql:///lampl?host=/tmp` | No user, so the role becomes `anonymous` (Postgres: `role "anonymous" does not exist`) |
| `postgresql://<user>@/lampl?host=/tmp` | `error with configuration: empty host`; the `url` crate rejects credentials with an empty host |

Where `anonymous` comes from: with no user in the URL and no `PGUSER`, sqlx 0.9 takes the user from the `whoami` crate, and sqlx builds `whoami` without its `std` feature, so `whoami` returns the constant `anonymous` on every platform, not only under launchd. `lampl serve` refuses an empty or `anonymous` role at start, before connecting, with a message naming `postgresql://<user>@%2Ftmp/lampl`. Setting `PGUSER` also works, unless it is `anonymous`.

4. Point a client at it:

   ```sh
   lampl init --server http://127.0.0.1:7878 --api-key-file ~/.config/lampl/laptop.key
   lampl doctor
   ```

## Operating it

| Task | Command or fact |
| --- | --- |
| Add or revoke a client | `lampl serve keys add NAME`, `keys revoke NAME`, `keys list`; the running server re-reads the file within a second |
| Check health | `GET /livez` is open; `GET /health` needs a key and reports vector coverage, embed mode and `degraded` |
| Bind beyond localhost | `--listen 0.0.0.0:7878`. There is no TLS in the server; put a reverse proxy in front before exposing it |
| Logs | `RUST_LOG=debug` or another `tracing` filter; default `info` |
| Back up | `lampl export -o backup.ndjson` plus your usual Postgres backup |
| Upgrade | Replace the binary and restart; migrations run at start; a database newer than the binary is refused |
| Multiple fallbacks | `lampl init --server URL --server FALLBACK` tries URLs in order until one answers |

## Embedding

By default the server embeds in process. To use an HTTP endpoint instead, see [../embeddings.md](../embeddings.md). A background worker embeds new rows every `--embed-interval-secs` (default 5), so a
fresh memory is found by full text immediately and by vector shortly after.
