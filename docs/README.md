# lamplighter documentation

lamplighter is agent memory as one binary: `lampl` stores decisions, root causes and gotchas, recalls them with hybrid full-text and vector search, and reaches an agent through hooks, a CLI and an MCP
server. This index lists every page. Agents can start from [llms.txt](../llms.txt).

## Start here

| Page | Read it to |
| --- | --- |
| [install.md](install.md) | Install `lampl` four ways, upgrade, uninstall |
| [quickstart.md](quickstart.md) | Store and recall a memory in five minutes, locally and against a server |
| [concepts.md](concepts.md) | Understand memory types, links, projects, tags and supersession |
| [best-practices.md](best-practices.md) | Decide what to store, how to tag, and how to retire stale memories |

## Run modes

| Page | Mode |
| --- | --- |
| [modes/README.md](modes/README.md) | Choose a mode |
| [modes/embedded-sqlite.md](modes/embedded-sqlite.md) | Embedded SQLite, no server (`lampl init --local`) |
| [modes/serve-postgres.md](modes/serve-postgres.md) | `lampl serve` over Postgres with pgvector |
| [modes/external-postgres.md](modes/external-postgres.md) | Pointing `lampl serve` at a Postgres you already run |
| [modes/docker-compose.md](modes/docker-compose.md) | The Docker image and `deploy/compose.yaml` |

## Reference

| Page | Covers |
| --- | --- |
| [cli.md](cli.md) | Every client command and flag |
| [api.md](api.md) | The `lampl serve` HTTP API |
| [configuration.md](configuration.md) | Every environment variable, flag and config file key |
| [mcp.md](mcp.md) | The `lampl mcp` tools |
| [plugin.md](plugin.md) | The Claude Code plugin, hook behaviour and its variables |
| [local-mode.md](local-mode.md) | Embedded store internals and Postgres parity |

## How it works

| Page | Covers |
| --- | --- |
| [ranking.md](ranking.md) | Fusion, the relevance floor and how it was calibrated |
| [embeddings.md](embeddings.md) | In-process Qwen3 versus an HTTP endpoint |
| [benchmarks.md](benchmarks.md) | Every published number, its method and its caveats |

## Operate and contribute

| Page | Covers |
| --- | --- |
| [troubleshooting.md](troubleshooting.md) | Symptoms, fixes and FAQ |
| [migrating-from-memorygraph.md](migrating-from-memorygraph.md) | Importing a memorygraph store |
| [releasing.md](releasing.md) | How a release is cut |
| [experiments/pgrx.md](experiments/pgrx.md) | Embedding inside Postgres with pgrx (experiment, not supported) |
| [../CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup and checks |
| [../SECURITY.md](../SECURITY.md) | Reporting a vulnerability |
