# Configuration

Every setting, with where it is read. Verified against `crates/lampl/src` at 0.1.0 and `lampl --help`.

## Client environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `LAMPL_SERVER` | | Comma-separated server URLs, tried in order. Wins over `config.toml`, so a machine with no config can use a server |
| `LAMPL_API_KEY` | | API key for server mode. Wins over `api_key_file` |
| `LAMPL_CONFIG_DIR` | platform config dir + `lampl` | Where `config.toml` lives |
| `LAMPL_DATA_DIR` | platform data dir + `lampl` | Where the embedded `lampl.db` lives |
| `LAMPL_EMBED_TIMEOUT_MS` | none | Abandon embedding a query after this long and recall by full text (local mode and `recall`) |
| `LAMPL_FROM_POSTGRES_URL` | | memorygraph database URL for `lampl import --from-postgres`; keeps the password out of argv |

## Server environment and flags (`lampl serve`)

| Flag | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `--database-url` | `LAMPL_DATABASE_URL` | required | Postgres URL. Prefer the environment |
| `--keys-file` | `LAMPL_KEYS_FILE` | required | `<client> <sha256-hex>` lines, mode 600. Refuses to start missing, empty or group/world readable |
| `--listen` | | `127.0.0.1:7878` | Address to bind |
| `--embed-endpoint` | | in-process model | HTTP embedding endpoint |
| `--embed-endpoint-kind` | | `openai` | `openai` or `ollama` |
| `--embed-model` | | `Qwen/Qwen3-Embedding-0.6B` | Model name sent to the endpoint |
| | `LAMPL_EMBED_API_KEY` | none | Bearer token for the endpoint |
| `--min-coverage` | | `0.95` | `/health` reports degraded below this vector coverage (0 to 1) |
| `--embed-interval-secs` | | `5` | Seconds between background embedding sweeps |
| | `RUST_LOG` | `info` | `tracing` filter for server logs |
| | `NO_COLOR` | unset | Any non-empty value forces plain log lines |

`lampl serve` writes its logs to stderr, never stdout. Lines carry ANSI colour only when stderr is a terminal and `NO_COLOR` is unset or empty, so a service manager (a launchd `StandardErrorPath`, systemd's journal) receives plain text. Capture stderr, not stdout.

`lampl serve keys add|list|revoke NAME` takes `--keys-file` or `LAMPL_KEYS_FILE`.

## Plugin hook (Claude Code)

| Variable | Default | Meaning |
| --- | --- | --- |
| `LAMPL_PUSH` | on | `0` turns the push off |
| `LAMPL_PUSH_TIMEOUT` | `4` | Total budget in seconds |
| `LAMPL_PUSH_RECALL_CAP` | `3.5` | Outer kill for the recall process, seconds |
| `LAMPL_PUSH_EMBED_MS` | `1500` | Embed timeout passed to `lampl recall` |
| `LAMPL_PUSH_BREAKER` | `300` | Circuit-breaker window, seconds |
| `LAMPL_PUSH_SKIP_WORDS` | `3` | Skip prompts with this many content words or fewer; `0` disables |
| `LAMPL_PUSH_STATE`, `LAMPL_PUSH_LOG` | temp dir, off | Session state directory; JSON-lines decision log |

## Config file

`config.toml` in the config directory, written by `lampl init`. Two shapes seen in practice:

```toml
mode = "local"
db_path = "/path/to/lampl.db"
```

```toml
mode = "server"
url = "http://127.0.0.1:7878"
# fallback_urls = ["http://backup:7878"]     # from `lampl init --server A --server B`
# api_key_file = "/path/to/key"               # from `--api-key-file PATH`
```

The key file holds the bare key; surrounding whitespace is trimmed. `init` refuses to switch an existing config between modes.

## Files and directories

| What | Where |
| --- | --- |
| Config | `$LAMPL_CONFIG_DIR/config.toml` (macOS: `~/Library/Application Support/lampl`, Linux: `~/.config/lampl`) |
| Embedded database | `$LAMPL_DATA_DIR/lampl.db` (Linux default `~/.local/share/lampl`) |
| Model weights | platform cache dir + `lamplighter/models` (macOS `~/Library/Caches`, Linux `~/.cache`) |

The platform defaults come from the `dirs` crate; the Linux paths were observed inside the container and the macOS paths on this machine.

## Benchmark and test variables

`LAMPL_API_KEY` is also read by `lampl-bench`. `LAMPL_TEST_DATABASE_URL` points the Postgres-backed tests at a pgvector superuser (see [CONTRIBUTING.md](../CONTRIBUTING.md) and [cli.md](cli.md)).

### Benchmark overrides

Read once by `lampl serve` at the first recall; local mode reads neither. They exist for floor sweeps ([ranking.md](ranking.md)), not for production.

| Variable | Default | Meaning |
| --- | --- | --- |
| `LAMPL_RECALL_FLOOR` | cosine 0.52, coverage 0.52 | `off`, or `<cosine>,<coverage>` thresholds; keeps the coverage norm |
| `LAMPL_RECALL_COVERAGE` | `capped:3` | How the floor's coverage is computed: `plain`, `salient:<k>` or `capped:<k>`; an unparsable value is logged and ignored |

`LAMPL_RECALL_COVERAGE=plain LAMPL_RECALL_FLOOR=0.52,0.35` reproduces the floor before LAMP-22 (cosine 0.52, plain coverage 0.35).
