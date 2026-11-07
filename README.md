<div align="center">
<h1>lamplighter</h1> </div>

**agent memory as one binary.**

`lampl` remembers what your coding agent learns (decisions, root causes, gotchas) and pushes the right memory back into the next session, with no LLM key and no graph database. Apache-2.0, written in
Rust, three run modes, and a Claude Code plugin that wires hooks, MCP and a skill together.

[![ci](https://github.com/moneymikeMD/lamplighter/actions/workflows/ci.yml/badge.svg)](https://github.com/moneymikeMD/lamplighter/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/moneymikeMD/lamplighter)](https://github.com/moneymikeMD/lamplighter/releases)
[![crates.io](https://img.shields.io/crates/v/lampl)](https://crates.io/crates/lampl)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

- **finds the memory a session needs** — on a 35-item gold set over 1,469 real memories, the hybrid full-text plus vector design lamplighter productises scored hook recall@5 of 0.961, against 0.913
  for mem0, 0.811 for Graphiti and 0.562 for ripgrep over markdown.
- **remembers without an LLM** — storing costs nothing but a local embedding. In the same benchmark mem0 cost $0.0135 and Graphiti $0.0516 per ingested memory, because both call a model to ingest.
- **says nothing when nothing is relevant** — a calibrated relevance floor lifted abstain-correct from 0 of 19 to 9 of 19 no-answer queries while recall@5 rose from 0.828 to 0.840. In Stage A, no arm
  abstained.
- **changes what the agent gets right** — headless Claude Code sessions pushed the top five memory titles went from 12.9% to 78.5% correct on Opus 5.5 and from 8.6% to 52.7% on Sonnet 5, against no
  memory at all.
- **one binary, three ways to run it** — embedded SQLite on your laptop, `lampl serve` over Postgres with pgvector for a team, or an external Postgres you already run. The release binary is about 22
  MB.

Every number above is a measurement on a private gold set from one author's store, with the method and the caveats in [Benchmarks](#benchmarks). The public LongMemEval run is not finished, and this
README says so there.

## What this is

A memory store for coding agents, built around one idea: an agent should be told what it already knows *before* it repeats a mistake, and should be told nothing when it knows nothing.

- **Hybrid recall.** Postgres full-text and pgvector cosine (or SQLite FTS5 and sqlite-vec) each return candidates, fused by reciprocal rank, then cut by a floor calibrated to return nothing for
  unrelated queries.
- **Embeddings in the process.** Qwen3-Embedding-0.6B runs on CPU inside `lampl` through candle. No Ollama, no API key. An HTTP embedding endpoint (OpenAI-style or Ollama) is optional.
- **Typed memories and links.** Six memory types, four link types, and `SUPERSEDES` retires a stale memory from recall instead of deleting it.
- **Fails closed.** A client configured for a server that is down exits non-zero; it never silently opens an empty local store.
- **Three doors for the agent.** Hooks push memories in, a CLI and an MCP server pull them out, and a skill teaches the agent when to store.

## Install

| Route | Command |
| --- | --- |
| Shell installer | `curl --proto '=https' --tlsv1.2 -LsSf https://github.com/moneymikeMD/lamplighter/releases/latest/download/lampl-installer.sh \| sh` |
| Cargo | `cargo install lampl` (needs Rust 1.99 or newer) |
| Docker | `docker pull ghcr.io/moneymikemd/lamplighter` (linux/amd64, linux/arm64) |
| Claude Code plugin | `claude plugin marketplace add moneymikeMD/moneymike-plugins` then `claude plugin install lamplighter@moneymike-plugins` |

The installer ships `lampl` for Apple Silicon, Linux x86_64 and Linux arm64 and puts it in `$CARGO_HOME/bin`. The plugin needs `lampl` on `PATH` and `python3` for the hook. Details:
[docs/install.md](docs/install.md).

## Quickstart

No server, no key, no model account. This was run against a local build of 0.1.0.

```sh
lampl init --local --prepare      # creates the SQLite store; downloads the embedding model once (about 1.1 GB)

lampl store --type decision --project demo --tags auth \
  --title "Use Postgres for the session store" \
  --body "Chose Postgres over Redis: sessions must survive restarts and we already run Postgres."
lampl store --type gotcha --project demo --tags auth \
  --title "Redis eviction drops sessions" \
  --body "maxmemory-policy allkeys-lru silently evicted live sessions in staging."

lampl recall "why did we pick postgres over redis for sessions" --project demo
lampl recall "kubernetes ingress timeout" --project demo
```

The first recall prints one JSON hit per line, each with `cosine`, `coverage`, `score` and the `memory`. The second prints nothing, and `--json` makes that explicit as `{"hits":[],"warning":null}`.
That empty answer is the relevance floor working. Next steps: [docs/quickstart.md](docs/quickstart.md).

## Choose your mode

| Mode | Store | Good for | Run it |
| --- | --- | --- | --- |
| [Embedded](docs/modes/embedded-sqlite.md) | SQLite, FTS5 and sqlite-vec | One person, one machine, nothing to operate | `lampl init --local` |
| [Serve + Postgres](docs/modes/serve-postgres.md) | Postgres 17 with pgvector | A team or several machines sharing one memory | `lampl serve` |
| [External Postgres](docs/modes/external-postgres.md) | A Postgres you already run | Managed databases, existing backups | `lampl serve --database-url ...` |
| [Docker / compose](docs/modes/docker-compose.md) | `pgvector/pgvector` container | A server up in a few commands | `docker compose -f deploy/compose.yaml up -d` |

Embedded mode only offers `store` and `recall`. Everything else (links, audit, export, import) needs a server. See [docs/modes/README.md](docs/modes/README.md).

## How recall works

1. The query is embedded in the process (or by the configured endpoint). If embedding fails or times out, recall continues by full text and says so in `warning`.
2. Full-text and vector search each return their top 50 candidates among memories that no `SUPERSEDES` link has retired.
3. Reciprocal-rank fusion (`score = Σ 1 / (60 + rank)`) merges the two lists.
4. A candidate survives the floor when its cosine is at least 0.70 **or** its IDF-weighted term coverage is at least 0.24. Otherwise it is dropped, and an empty list is a valid answer.

The floor was calibrated, not guessed: see [docs/ranking.md](docs/ranking.md) for the sweep and [docs/concepts.md](docs/concepts.md) for the model.

## Claude Code plugin

The plugin makes memory ambient. On `SessionStart`, `UserPromptSubmit` and after a failed Bash call, the hook runs `lampl recall --json` and injects the top matches, each memory once per session. It
has a 3 second budget, a circuit breaker, and prints a one-line message when memory is broken rather than pretending it is empty.

| MCP tool | Purpose |
| --- | --- |
| `store` | Save a memory with a typed `type` enum |
| `recall` | Hybrid search, same ranking and floor as the hook |
| `get` | Read one memory by id (server mode) |
| `link` | Connect two memories with a typed link (server mode) |
| `related` | List a memory's links in both directions (server mode) |

A skill tells the agent what to store, what not to, and how to supersede a stale memory. Details: [docs/plugin.md](docs/plugin.md), [docs/mcp.md](docs/mcp.md).

## Memory types and links

Types: `fact` (default), `decision`, `problem`, `fix`, `procedure`, `gotcha`. Links: `SUPERSEDES`, `SOLVES`, `CAUSES`, `CONTRADICTS`.

```sh
PROBLEM=$(lampl store --type problem --title "Deploys hang at migrate" --body "..." | jq -r .id)
FIX=$(lampl store --type fix --title "Run migrations before the rollout" --body "..." | jq -r .id)
lampl link "$FIX" "$PROBLEM" SOLVES        # server mode
```

A memory that is the target of `SUPERSEDES` stops appearing in recall and in `audit`. More: [docs/concepts.md](docs/concepts.md) and [docs/best-practices.md](docs/best-practices.md).

## Benchmarks

| Measurement | Result | Method and caveat |
| --- | --- | --- |
| Retrieval, hook recall@5 (Stage A) | postgres_hybrid 0.961, mem0 0.913, Graphiti 0.811, ripgrep 0.562 | Private 35-item gold set (31 answerable, 4 abstain) over a frozen 1,469-memory corpus, 208 queries, zero API calls. Scores are case-averaged. `postgres_hybrid` is the design lamplighter productises, not `lampl` itself. |
| Ingest cost | mem0 $0.0135, Graphiti $0.0516 per memory | Haiku 4.5 list price over all 1,469 memories, failed attempts included. lamplighter makes no LLM call. |
| Relevance floor (LAMP-19) | recall@5 0.828 to 0.840, abstain-correct 0/19 to 9/19 | Private 138-query set over 2,658 memories. Only 19 no-answer queries, so one query moves the result 5 points. |
| End to end (Stage B) | correct 12.9% to 78.5% (Opus 5.5), 8.6% to 52.7% (Sonnet 5) | 4,117 headless Claude Code sessions, 31 items, 3 repeats, a Haiku 4.5 judge. Questions come from the same author's incident history. Detectable difference is about 20 points. |
| Public LongMemEval | **not run in full** | `lampl-bench` exists and a 6-question smoke run scored recall@5 0.667. That is a harness check, not a result. |

All gold sets are private, so only aggregates are published. mem0 and Graphiti were run by us on our corpus; their own published numbers are self-reported and measure different things, so none are
repeated here. Details and the reproduce command: [docs/benchmarks.md](docs/benchmarks.md).

## Feature comparison

| | lamplighter | mem0 | Graphiti |
| --- | --- | --- | --- |
| License | Apache-2.0 | Apache-2.0 | Apache-2.0 |
| Language | Rust, one binary | Python | Python |
| LLM required to store | No | Yes, per its README ([source](https://github.com/mem0ai/mem0#readme)) | Yes, an OpenAI key by default ([source](https://github.com/getzep/graphiti#readme)) |
| Store | SQLite or Postgres with pgvector | Not compared here | A graph database (Neo4j, FalkorDB, Neptune or Kuzu) |
| Returns nothing for an irrelevant query | Yes, a calibrated floor | No: returned 5 results on all 4 abstain items in our Stage A run | No: same |
| Claude Code plugin with hooks, MCP and skill | Yes | Not compared here | Not compared here |

Licenses are from the GitHub API on 2026-11-07. "Not compared here" means we did not check, not that the answer is no. Other projects in this space, including Cognee, claude-mem and basic-memory, are
surveyed in [docs/benchmarks.md](docs/benchmarks.md) only for how they report numbers.

## Migrating from memorygraph

```sh
# needs a configured server
lampl import ~/.memorygraph/export-final-2026-09-28.json --dry-run
lampl import ~/.memorygraph/export-final-2026-09-28.json
```

Types are mapped, ids are kept, vectors are re-embedded, and a second run adds nothing. `RELATED_TO`, `SIMILAR_TO` and `BLOCKS` links have no equivalent and are dropped with a count. See
[docs/migrating-from-memorygraph.md](docs/migrating-from-memorygraph.md).

## Docs

Start at [docs/README.md](docs/README.md). Agents can read [llms.txt](llms.txt).

| Topic | Page |
| --- | --- |
| Install, quickstart | [install](docs/install.md), [quickstart](docs/quickstart.md) |
| Concepts, best practices | [concepts](docs/concepts.md), [best practices](docs/best-practices.md) |
| Run modes | [modes](docs/modes/README.md) |
| Reference | [CLI](docs/cli.md), [HTTP API](docs/api.md), [configuration](docs/configuration.md), [MCP](docs/mcp.md), [plugin](docs/plugin.md) |
| How it works | [ranking](docs/ranking.md), [embeddings](docs/embeddings.md), [benchmarks](docs/benchmarks.md) |
| Help | [troubleshooting and FAQ](docs/troubleshooting.md) |

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md). The short version: install `lefthook`, `typos-cli`, `cargo-deny` and `cargo-shear`, run `lefthook install`, and expect `cargo fmt`, `cargo clippy -D warnings`
and `cargo test --workspace` to gate your change. Commits are conventional (`feat:`, `fix:`, `docs:`); breaking markers are rejected while the project is pre-1.0, and releases are cut by
release-please ([docs/releasing.md](docs/releasing.md)).

## Security

lamplighter has no telemetry and no accounts. `lampl serve` refuses to start without API keys, stores only SHA-256 hashes of them, refuses a keys file readable by group or others, and re-reads the
file within a second so a revoked key stops working without a restart. Keep secrets out of memories; the store is not encrypted at rest. Report a vulnerability through the repository's Security tab
or, if that is not offered, by opening an issue that asks for a private contact without describing the flaw. See [SECURITY.md](SECURITY.md).

## Telemetry

None. The code contains no analytics, crash reporting or update checks. The only outbound connections are the ones you configure or ask for: the client to your `lampl serve` URL, `lampl serve` to your
Postgres and to an embedding endpoint if you set one, a one-time Hugging Face download of the embedding weights when you run `lampl init --local --prepare` or `lampl doctor --fetch-model`, and the
LongMemEval dataset download in `lampl-bench`.

## License

Apache-2.0. See [LICENSE](LICENSE).

## Lineage

lamplighter is a fresh Rust codebase, but the ideas and some ported code come from two places:

- [memory-graph/memory-graph](https://github.com/memory-graph/memory-graph) (MIT), through the owner's fork [moneymikeMD/memory-graph](https://github.com/moneymikeMD/memory-graph), based on v0.14.0.
- The memory-push hooks in [night-watchman](https://github.com/moneymikeMD/night-watchman) (MIT), which inject matching memories at session start and on each prompt.

Ported code keeps its original MIT notice, recorded in the [NOTICE](NOTICE) file.
