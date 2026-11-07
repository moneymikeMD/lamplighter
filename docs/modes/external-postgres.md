# External Postgres

Use this when a Postgres already exists (a managed service, a shared instance with backups) and you want `lampl serve` to use it instead of a container. The server is the same as in
[serve-postgres.md](serve-postgres.md); only the database changes.

## What the database must provide

- The `vector` extension, available to install. The first migration runs `CREATE EXTENSION IF NOT EXISTS vector`.
- A database and a role that owns it, with the right to create tables and indexes in the target schema (`public`).
- pgvector with `halfvec` support (0.7 or newer; UNVERIFIED lower bound).
- A server-side connection from wherever `lampl serve` runs. Clients never connect to the database; they talk to `lampl serve`.

## Least-privilege setup

Create the extension as a superuser once, so the application role does not need that right. Run as a superuser:

```sql
CREATE DATABASE lamplighter;
CREATE ROLE lampl LOGIN PASSWORD '...';
\c lamplighter
CREATE EXTENSION vector;
GRANT ALL ON SCHEMA public TO lampl;
ALTER DATABASE lamplighter OWNER TO lampl;
```

Tested with `pgvector/pgvector:pg17`: with the extension pre-created, `lampl serve` connected as the non-superuser `lampl`, applied its migrations (logging `extension "vector" already exists,
skipping`) and started. Whether a non-superuser can create the extension itself depends on your provider (many managed services allow it for a designated role); that case was not tested here
(UNVERIFIED).

## Run the server

```sh
export LAMPL_DATABASE_URL='postgres://lampl:...@db.example.internal:5432/lamplighter?sslmode=require'
export LAMPL_KEYS_FILE=/etc/lampl/keys
lampl serve keys add laptop
lampl serve --listen 127.0.0.1:7878
```

`PgStore` connects with `sqlx` and rustls TLS. Pass TLS options in the URL as your provider documents (UNVERIFIED for specific providers).

## Sharing a database

The tables are `lampl`'s; do not point two incompatible versions at one database. A server refuses to start when the schema version is newer than it understands. To keep several projects apart inside
one store, use `--project`, not several databases.

## Migrating into it

Import a lamplighter export with `lampl import FILE` (vectors are restored without re-embedding if the model id and digest match). Import from memorygraph's tables directly with `lampl import
--from-postgres URL`; see [../migrating-from-memorygraph.md](../migrating-from-memorygraph.md).
