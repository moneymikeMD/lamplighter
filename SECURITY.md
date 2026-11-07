# Security

## Reporting a vulnerability

Please do not describe a vulnerability in a public issue. Use GitHub's "Report a vulnerability" button on the repository's Security tab if it is offered. If it is not, open an issue that says you have
a security report and asks for a private way to send it, without any detail of the flaw, and the maintainer will arrange one.

## Model

- `lampl serve` is the only process that holds the database password and the embedder. It refuses to start without a keys file, with an empty one, or with one readable by group or others.
- API keys are `lampl_` plus 64 hex characters, shown once; only their SHA-256 is stored. `GET /livez` is the only unauthenticated endpoint. A malformed keys file fails closed.
- The server has no TLS and no rate limiting; bind it to localhost or put a reverse proxy in front.
- Memories are stored in plaintext in SQLite or Postgres. Do not store secrets in them.
- There is no telemetry; see the README.

Supported versions: the latest release.
